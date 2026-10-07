//! The app core (D22): the event loop that feeds the reducer and draws the UI.

use std::time::Duration;

use anyhow::Context as _;
use crossterm::event::{Event, EventStream};
use faro_core::backend::{Backend, BackendEvents};
use faro_core::domain::{Contact, Group, ServiceId};
use faro_core::store::Store;
use futures_util::StreamExt as _;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

use crate::config::{self, Startup};
use crate::theme::{ColorDepth, Themes};
use crate::tui::Tui;
use crate::ui::{self, View};

pub mod commands;
mod effects;
pub mod event;
pub mod input;
pub mod keymap;
pub mod palette;
pub mod state;
pub mod system;
mod timeline;
pub mod update;

use event::{AppEvent, Effect};
use state::State;

/// At most this often (about 60 fps); floods of events are coalesced into one frame.
const FRAME: Duration = Duration::from_millis(16);

/// Who we are and who we know, loaded before the UI starts.
pub struct Directory {
    pub me: ServiceId,
    pub contacts: Vec<Contact>,
    pub groups: Vec<Group>,
}

/// Read the account, contacts and groups from the backend (local data, no
/// network) and copy contacts and groups into the store.
pub async fn load_directory<B: Backend>(backend: &B, store: &Store) -> anyhow::Result<Directory> {
    let account = backend.whoami().await.context("reading the account")?;
    let contacts = backend.contacts().await.context("reading contacts")?;
    let groups = backend.groups().await.context("reading groups")?;
    tracing::info!(
        account = %account.service_id,
        contacts = contacts.len(),
        groups = groups.len(),
        capabilities = ?backend.capabilities(),
        "backend ready"
    );
    store.upsert_contacts(contacts.clone()).await?;
    store.upsert_groups(groups.clone()).await?;
    Ok(Directory {
        me: account.service_id,
        contacts,
        groups,
    })
}

/// Run the TUI until the user quits or faro is told to stop. Must run inside a
/// `LocalSet`: the backend is driven by a local task (`commands.rs`).
pub async fn run<B: Backend + 'static>(
    backend: B,
    mut backend_events: BackendEvents,
    store: Store,
    directory: Directory,
    tz: jiff::tz::TimeZone,
    emoji: state::EmojiSetting,
    startup: Startup,
) -> anyhow::Result<()> {
    let Startup {
        path: config_path,
        config: settings,
        mut problems,
    } = startup;
    let mut terminate = signal(SignalKind::terminate()).context("listening for SIGTERM")?;
    let mut hangup = signal(SignalKind::hangup()).context("listening for SIGHUP")?;
    let (results_tx, mut results) = mpsc::unbounded_channel();
    let (effects, worker) = effects::spawn(store, results_tx.clone());
    let (commands, command_task) = commands::spawn(backend, results_tx.clone());
    let (config_writer, config_task) =
        config::spawn_writer(config_path.clone(), results_tx.clone());
    // Hot reload (D48); without it, faro still works with what it read.
    let watch = config::try_watch(&config_path, results_tx.clone(), &mut problems);
    let router = Router {
        commands,
        effects,
        config: config_writer,
        results: results_tx,
    };
    let dispatch = |list: Vec<Effect>| router.dispatch(list);

    let mut state = State::new(directory.me, directory.contacts, directory.groups);
    state.env.over_ssh = system::over_ssh();
    state.env.emoji = emoji;
    state.env.colors = ColorDepth::detect(|var| std::env::var(var).ok());
    tracing::info!(colors = ?state.env.colors, "colour depth detected");
    dispatch(update::configure(&mut state, settings, problems));
    let mut tui = Tui::enter(state.config.mouse.enabled).context("setting up the terminal")?;
    let mut input = EventStream::new();
    dispatch(vec![Effect::LoadConversations]);

    let mut themes = Themes::default();
    let mut dirty = true;
    let mut last_draw: Option<Instant> = None;
    loop {
        if tui.mouse() != state.config.mouse.enabled {
            tui.set_mouse(state.config.mouse.enabled)
                .context("switching the mouse")?;
        }
        if dirty && last_draw.is_none_or(|t| t.elapsed() >= FRAME) {
            let geometry = draw(&mut tui, &mut state, &mut themes, &tz)?;
            last_draw = Some(Instant::now());
            dirty = false;
            // Same picture; only bookkeeping (and maybe loading older messages).
            dispatch(update::update(&mut state, AppEvent::Drawn(geometry)));
        }
        if state.quit {
            break;
        }
        let redraw_at = last_draw.map_or_else(Instant::now, |t| t + FRAME);
        let deadline = state.next_deadline();
        let event = tokio::select! {
            terminal = input.next() => match terminal {
                Some(Ok(event)) => AppEvent::Terminal(event),
                Some(Err(error)) => {
                    tracing::error!(%error, "terminal input failed");
                    AppEvent::Shutdown
                }
                None => AppEvent::Shutdown,
            },
            Some(event) = backend_events.recv() => AppEvent::Backend(event),
            Some(event) = results.recv() => event,
            () = sleep_until(redraw_at), if dirty => continue,
            () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                AppEvent::Tick
            }
            _ = terminate.recv() => AppEvent::Shutdown,
            _ = hangup.recv() => AppEvent::Shutdown,
        };
        dirty |= affects_view(&event);
        dispatch(update::update(&mut state, event));
    }

    drop(tui);
    drop(watch);
    // A send still in flight is abandoned; the store marks it failed on the
    // next start (`Store::fail_unsent`).
    command_task.abort();
    let Router {
        effects, config, ..
    } = router;
    // Let queued writes finish before the caller closes the store.
    drop(effects);
    finish(worker, "effects worker").await;
    // Same for a setting changed just before quitting.
    drop(config);
    finish(config_task, "config writer").await;
    Ok(())
}

/// Where effects go: store work to the effects worker, backend commands to
/// the command task, settings to the config writer; terminal and system
/// effects run here, on the app loop.
struct Router {
    commands: mpsc::UnboundedSender<commands::Command>,
    effects: mpsc::UnboundedSender<Effect>,
    config: mpsc::UnboundedSender<faro_core::config::SettingEdit>,
    results: mpsc::UnboundedSender<AppEvent>,
}

impl Router {
    fn dispatch(&self, list: Vec<Effect>) {
        for effect in list {
            let delivered = match effect {
                Effect::Backend(command) => self.commands.send(command).is_ok(),
                Effect::Copy(text) => {
                    system::copy(&text);
                    true
                }
                Effect::OpenUrl(url) => {
                    system::open_url(url, self.results.clone());
                    true
                }
                Effect::CheckTerminal => {
                    system::check_terminal(self.results.clone());
                    true
                }
                Effect::Bell => {
                    system::bell();
                    true
                }
                Effect::SaveSetting(edit) => self.config.send(edit).is_ok(),
                store @ (Effect::LoadConversations
                | Effect::LoadMessages(_)
                | Effect::LoadOlder { .. }
                | Effect::Persist(_)
                | Effect::MarkRead(_)
                | Effect::SaveConversationState(..)
                | Effect::RememberOpen(_)) => self.effects.send(store).is_ok(),
            };
            if !delivered {
                tracing::error!("a worker task stopped; dropping an effect");
            }
        }
    }
}

/// Wait for a worker to finish its queue.
async fn finish(task: tokio::task::JoinHandle<()>, name: &str) {
    if let Err(error) = task.await {
        tracing::error!(%error, "{name} failed");
    }
}

/// Draw a frame (clearing the screen first if asked to) and report where
/// things ended up.
fn draw(
    tui: &mut Tui,
    state: &mut State,
    themes: &mut Themes,
    tz: &jiff::tz::TimeZone,
) -> anyhow::Result<state::Geometry> {
    // Unknown names were reported when the config was read.
    let theme = themes.get(&state.config.theme, state.colors());
    let view = View {
        theme,
        tz,
        now: jiff::Timestamp::now(),
    };
    let mut geometry = state::Geometry::default();
    if std::mem::take(&mut state.env.repaint) {
        tui.clear().context("clearing the screen")?;
    }
    tui.draw(|frame| geometry = ui::render(frame, state, &view))
        .context("drawing")?;
    // Follows what's on screen, so it's checked once per frame (D49).
    tui.set_title(state.window_title(view.now_timestamp()).as_deref())
        .context("setting the window title")?;
    Ok(geometry)
}

/// Whether an event can change what's on screen; the rest don't cost a frame.
fn affects_view(event: &AppEvent) -> bool {
    match event {
        AppEvent::Terminal(Event::Mouse(mouse)) => update::mouse_affects_view(*mouse),
        AppEvent::Drawn(_) => false,
        // Focus changes can clear unread counts (D42).
        AppEvent::Terminal(
            Event::Key(_)
            | Event::Paste(_)
            | Event::Resize(..)
            | Event::FocusGained
            | Event::FocusLost,
        )
        | AppEvent::Backend(_)
        | AppEvent::Loaded { .. }
        | AppEvent::MessagesLoaded { .. }
        | AppEvent::OlderLoaded { .. }
        | AppEvent::SendResult { .. }
        | AppEvent::ReactResult { .. }
        | AppEvent::OpenFailed(_)
        | AppEvent::TerminalChecked { .. }
        | AppEvent::ConfigRead(_)
        | AppEvent::SettingNotSaved(_)
        | AppEvent::StoreFailed(_)
        | AppEvent::Tick
        | AppEvent::Shutdown => true,
    }
}
