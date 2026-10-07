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

use crate::theme::Theme;
use crate::tui::Tui;
use crate::ui::{self, View};

pub mod commands;
mod effects;
pub mod event;
pub mod input;
pub mod state;
mod system;
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
) -> anyhow::Result<()> {
    let mut terminate = signal(SignalKind::terminate()).context("listening for SIGTERM")?;
    let mut hangup = signal(SignalKind::hangup()).context("listening for SIGHUP")?;
    let (results_tx, mut results) = mpsc::unbounded_channel();
    let (effects, worker) = effects::spawn(store, results_tx.clone());
    let (commands, command_task) = commands::spawn(backend, results_tx.clone());
    let dispatch = |list: Vec<Effect>| {
        for effect in list {
            let delivered = match effect {
                Effect::Backend(command) => commands.send(command).is_ok(),
                Effect::Copy(text) => {
                    system::copy(&text);
                    true
                }
                Effect::OpenUrl(url) => {
                    system::open_url(url, results_tx.clone());
                    true
                }
                store @ (Effect::LoadConversations
                | Effect::LoadMessages(_)
                | Effect::LoadOlder { .. }
                | Effect::Persist(_)
                | Effect::MarkRead(_)
                | Effect::SaveConversationState(..)
                | Effect::RememberOpen(_)
                | Effect::SetAllowlistMode(_)) => effects.send(store).is_ok(),
            };
            if !delivered {
                tracing::error!("effects worker or command task stopped; dropping an effect");
            }
        }
    };

    let mut state = State::new(directory.me, directory.contacts, directory.groups);
    state.env.over_ssh = system::over_ssh();
    let theme = Theme::default();
    let mut tui = Tui::enter().context("setting up the terminal")?;
    let mut input = EventStream::new();
    dispatch(vec![Effect::LoadConversations]);

    let mut dirty = true;
    let mut last_draw: Option<Instant> = None;
    loop {
        if dirty && last_draw.is_none_or(|t| t.elapsed() >= FRAME) {
            let view = View {
                theme: &theme,
                tz: &tz,
                now: jiff::Timestamp::now(),
            };
            let mut geometry = state::Geometry::default();
            tui.draw(|frame| geometry = ui::render(frame, &state, &view))
                .context("drawing")?;
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
    // A send still in flight is abandoned; the store marks it failed on the
    // next start (`Store::fail_unsent`).
    command_task.abort();
    // Let queued writes finish before the caller closes the store.
    drop(effects);
    if let Err(error) = worker.await {
        tracing::error!(%error, "effects worker failed");
    }
    Ok(())
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
        | AppEvent::StoreFailed(_)
        | AppEvent::Tick
        | AppEvent::Shutdown => true,
    }
}
