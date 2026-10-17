//! The demo's starting point (D112, amended by D122): the mock's backlog in
//! the in-memory store, loaded through the real reducer, with the cast's
//! liveliest group chat open. That one picture is what the site's poster
//! shows *and* what the live demo starts on, so the handover from poster to
//! live app is seamless (WEBSITE.md §3.2, §6.4).

use std::time::Duration;

use faro_app::app::event::{AppEvent, Effect};
use faro_app::app::image_cache::NoImages;
use faro_app::app::state::{BlockGlyphs, EmojiMode, EmojiSetting, Focus, GraphicsSupport, State};
use faro_app::app::update::{self, PAGE, update};
use faro_app::theme::{ColorDepth, Theme};
use faro_app::ui::{self, View};
use faro_core::backend::{Backend as _, BackendEvent, BackendEvents, Capabilities};
use faro_core::config::InlineImages;
use faro_core::domain::{ConnectionState, ConversationId};
use faro_core::mock::{DETERMINISTIC_EPOCH, MockBackend, MockConfig, MockEpoch};
use faro_core::rt;
use faro_core::store::{Profile, Store};
use ratatui::Terminal;
use ratatui::buffer::Buffer;
use tokio::sync::mpsc::error::TryRecvError;

use crate::backend::CanvasBackend;
use crate::keys::demo_config;

/// The demo's seed: the same conversation for every visitor.
pub const SEED: u64 = 42;

/// How much backlog the demo starts with, bigger than the mock's default 60.
/// The mock plays whole scenes into the backlog and reshuffles when it runs
/// out of them, and [`CHAT`]'s scenes sit late in the deck — with 60 messages
/// (seed 42) the branch has no backlog at all. 400 covers all 30 scenes
/// (≈ 180 lines) at least once, so the branch's liveliness shows in the panel.
pub const BACKLOG: usize = 400;

/// The chat the demo opens first: the cast's biggest group, 15 members and
/// the most scenes (`mock/cast.rs`), named in the poster's alt-text.
pub const CHAT: &str = "Scranton Branch";

/// How often to look for more of the backlog, and how many empty looks in a
/// row mean it has all arrived (the mock sends it in one go after connecting).
const POLL: Duration = Duration::from_millis(100);
const QUIET_POLLS: u32 = 3;

pub type Error = Box<dyn std::error::Error>;

/// The mock as visitors get it (D-b): the demo's seed and backlog, today's
/// dates, the default activity rate, no scripted outages or errors.
#[must_use]
pub fn visitor_mock() -> MockConfig {
    MockConfig {
        seed: SEED,
        backlog: BACKLOG,
        disconnects: false,
        errors: false,
        ..MockConfig::default()
    }
}

/// The visitor's mock for recording the README demo (`tools/demo_gif`):
/// everyone answers every message within seconds, so the storyboard's
/// "write, and Jim answers" beat holds whatever the scenes become.
#[must_use]
pub fn recording_mock() -> MockConfig {
    MockConfig {
        eager_answers: true,
        ..visitor_mock()
    }
}

/// The same mock with the clock pinned and no live activity, so a frame of it
/// is byte-identical across runs and days: what the site poster runs (§6.4).
/// The backlog is dated *relative to the epoch* either way, so the relative
/// timestamps ("3h") the visitor sees are the poster's.
#[must_use]
pub fn poster_mock() -> MockConfig {
    MockConfig {
        epoch: MockEpoch::Fixed(DETERMINISTIC_EPOCH),
        activity: None,
        ..visitor_mock()
    }
}

/// A first frame: [`first_state`] drawn into a `width × height` buffer.
pub async fn first_frame(width: u16, height: u16) -> Result<Buffer, Error> {
    Ok(draw(&first_state().await?, width, height))
}

/// The state a visitor starts from, as the poster draws it: `poster_mock`'s
/// backlog in the memory store, [`CHAT`] open at the bottom, and the still
/// logo frame. The live demo builds the same state through
/// [`session::Session`] (D122); the test that the two agree is
/// `the_poster_is_the_live_demo_start`.
pub async fn first_state() -> Result<State, Error> {
    let (mut state, store) = stored_state(&poster_mock()).await?;
    if let Some((chat, _)) = open_start(&mut state) {
        // The effects `open_start` queues (loading history, marking read) are
        // what the live session's router dispatches; the poster loads the
        // history from the store itself and needs no store writes.
        load(&mut state, &store, chat).await?;
    }
    store.close().await?;
    Ok(state)
}

/// The same with `mock`, and nothing open: the store stays open so the
/// caller can open a conversation of its own the way the app does (the site
/// poster, §6.4). Keep the mock deterministic (`MockConfig::deterministic`,
/// no activity) if the frame has to repeat.
pub async fn stored_state(mock: &MockConfig) -> Result<(State, Store), Error> {
    let (backend, events) = MockBackend::new(mock);
    let me = backend.whoami().await?.service_id;
    let contacts = backend.contacts().await?;
    let groups = backend.groups().await?;
    let capabilities = backend.capabilities();
    let backlog = backlog(events).await;
    drop(backend);

    let store = Store::memory();
    store.upsert_contacts(contacts.clone()).await?;
    store.upsert_groups(groups.clone()).await?;
    for event in backlog {
        store.apply(event).await?;
    }
    let mut state = State::new(me, contacts, groups);
    // The demo's own settings and environment, as the live session sets them
    // (D114, D116): the poster's frame is the live demo's, so it runs with the
    // same emoji mode, colours and key map. What configuring returns goes to
    // the live session's router; there is nothing to write here.
    let _ = configure_visitor(&mut state, capabilities);
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: store.conversations().await?,
            last_open: None,
            recent_reactions: Vec::new(),
        },
    );
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Connection(ConnectionState::Connected)),
    );
    Ok((state, store))
}

/// The profile, environment and settings a visitor's faro runs with (D114,
/// D116, D118): the demo profile, the browser's capabilities, native block
/// glyphs so `bubble_style = "auto"` gives the modern look, half-block
/// images, truecolour, and emoji drawn as they are. Then the demo's own
/// config; the effects it returns go to the caller's router (D113).
pub fn configure_visitor(state: &mut State, capabilities: Capabilities) -> Vec<Effect> {
    state.profile = Profile::Demo;
    state.capabilities = capabilities;
    // The demo starts without the keyboard: the browser hands faro the keys
    // only when the canvas is the active element (D125). Both the live session
    // and the poster start from this, so the poster shows the same legitimately
    // unfocused picture the live demo settles on.
    state.env.focused = false;
    state.env.glyphs = BlockGlyphs::Native;
    // Images are pictures, drawn the way faro draws them in kitty (D135);
    // Settings › Inline images › halfblocks shows what other terminals get.
    state.env.graphics = GraphicsSupport::Direct;
    state.env.image_protocol = Some(InlineImages::Kitty);
    state.env.colors = ColorDepth::Truecolor;
    // The renderer draws each cell at faro's width: no terminal to work
    // around (D118). `auto` still follows the config like the terminal.
    state.env.emoji = EmojiSetting {
        mode: EmojiMode::Exact,
        auto: true,
        exact: true,
        ..EmojiSetting::default()
    };
    let (config, problems) = demo_config();
    update::configure(state, config, problems)
}

/// [`CHAT`] in the visitor's chat list, by title prefix (the full title
/// carries an emoji; "Scranton Business Park" doesn't match).
#[must_use]
pub fn chat(state: &State) -> Option<ConversationId> {
    state
        .visible_conversations()
        .find(|summary| {
            summary
                .title
                .as_deref()
                .is_some_and(|title| title.starts_with(CHAT))
        })
        .map(|summary| summary.id)
}

/// Open [`CHAT`] the way the app does — put the list cursor on it and run the
/// reducer's own open — and hand back the effects to dispatch (D113). `None`
/// while it isn't in the list yet: the backlog arrives a moment after the
/// store's first answer.
///
/// It does *not* drive the list with synthetic keys: the demo starts without
/// the keyboard (D125), and keys run through the reducer as if a focused
/// visitor typed them, claiming focus and marking the backlog read. Instead
/// the cursor is set directly and [`update::open_conversation`] opens the chat,
/// which leaves `env.focused` as it is. Opening normally marks the chat read;
/// since this open isn't a visitor action and faro has no keys, the unread
/// count is put back and the store write dropped — the visitor's first real
/// focus sends the read receipts.
///
/// The list keeps the focus and no message is selected: the live demo starts
/// unfocused (§4.2), so the poster must not show a selection tint either.
pub fn open_start(state: &mut State) -> Option<(ConversationId, Vec<Effect>)> {
    let target = chat(state)?;
    state.list.cursor = Some(target);
    if state.highlighted() != Some(target) {
        return None;
    }
    let mut effects = update::open_conversation(state, target);
    effects.retain(|effect| !matches!(effect, Effect::MarkRead(_)));
    if let Some(open) = &state.open {
        let unread = open.unread_on_open;
        if let Some(summary) = state.conversations.iter_mut().find(|c| c.id == target) {
            summary.unread = unread;
        }
    }
    state.focus = Focus::Conversations;
    Some((target, effects))
}

/// Load `conversation`'s history from `store` into the open conversation, the
/// page its own store worker loads ([`PAGE`]).
pub async fn load(
    state: &mut State,
    store: &Store,
    conversation: ConversationId,
) -> Result<(), Error> {
    update(
        state,
        AppEvent::MessagesLoaded {
            conversation,
            messages: store.messages(conversation, None, PAGE).await?,
        },
    );
    Ok(())
}

/// Everything the mock delivers on connecting: it says `Connected`, then
/// sends the backlog in one go.
async fn backlog(mut events: BackendEvents) -> Vec<BackendEvent> {
    let mut backlog = Vec::new();
    let mut connected = false;
    let mut quiet = 0;
    while quiet < QUIET_POLLS {
        match events.try_recv() {
            Ok(event) => {
                connected |= event == BackendEvent::Connection(ConnectionState::Connected);
                backlog.push(event);
                quiet = 0;
            }
            Err(TryRecvError::Empty) => {
                if connected {
                    quiet += 1;
                }
                rt::sleep(POLL).await;
            }
            Err(TryRecvError::Disconnected) => break,
        }
    }
    backlog
}

/// Draw `state` into a `width × height` buffer: the theme its config names
/// (resolved the way the app loop resolves it), the deterministic clock and
/// the still logo frame, so the picture is byte-identical across runs and
/// days. The first frame and the site poster both come through here, and both
/// run on a `poster_mock` state: `now` is the epoch that mock's backlog is
/// dated from, which is what keeps the relative timestamps ("3h") the same as
/// the live demo's, whose epoch is the real now.
pub fn draw(state: &State, width: u16, height: u16) -> Buffer {
    let theme = Theme::find(state.theme_name(), &state.user_themes)
        .cloned()
        .unwrap_or_default();
    let tz = jiff::tz::TimeZone::UTC;
    let now = i64::try_from(DETERMINISTIC_EPOCH.as_millis())
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    let view = View {
        theme: &theme,
        tz: &tz,
        now,
        logo_time: None,
        images: &NoImages,
    };
    let Ok(mut terminal) = Terminal::new(CanvasBackend::new(width, height));
    // `CanvasBackend` can't fail.
    let Ok(_) = terminal.draw(|frame| {
        ui::render(frame, state, &view);
    });
    terminal.backend().buffer().clone()
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use faro_app::app::event::Command;
    use faro_app::app::state::State;

    use super::*;
    use crate::router::Page;
    use crate::router::tests::FakePage;
    use crate::session::{Inbox, Session};

    /// The chat list has arrived and the branch is open with its history:
    /// where the live demo settles before its first frame.
    fn started(state: &State) -> bool {
        state
            .open
            .as_ref()
            .is_some_and(|open| !state.loading && !open.loading && !open.messages.is_empty())
    }

    /// Feed everything that arrives to the session until `what` holds, or
    /// fail: the demo's start has to settle on its own.
    async fn settle(session: &mut Session, inbox: &mut Inbox, what: impl Fn(&State) -> bool) {
        let until = rt::Instant::now() + Duration::from_secs(30);
        while !what(&session.state) {
            let event = tokio::select! {
                Some(event) = inbox.backend.recv() => AppEvent::Backend(event),
                Some(event) = inbox.results.recv() => event,
                () = rt::sleep_until(until) => panic!("the demo's start never settled"),
            };
            session.handle(event);
        }
    }

    /// A live session, started the way the page starts it.
    async fn live(mock: &MockConfig) -> (Session, Inbox) {
        let page = Rc::new(FakePage::default());
        let shared: Rc<dyn Page> = page;
        Session::start(mock, shared).await.unwrap()
    }

    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn the_first_frame_shows_the_branch_chat() {
        let buffer = first_frame(120, 34).await.unwrap();
        let text = faro_app::ui::text::grid(&buffer);
        // The chat list, and the branch open with its messages: the newest
        // ones sit at the bottom, nothing is selected (§4.2).
        assert!(text.contains("╭ Conversations"), "{text}");
        assert!(text.contains("╭ Scranton Branch"), "{text}");
        assert!(!text.contains("╭ Michael Scott"), "{text}");
        assert!(!text.contains("Loading…"), "{text}");
        assert!(!text.contains("connecting…"), "{text}");
    }

    /// The demo starts like an inbox you keep up with (D134): only the branch
    /// and Jim have a few unread messages, the ones after our last reply
    /// (D139); live messages bring the rest.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn the_demo_starts_with_two_chats_unread() {
        let state = first_state().await.unwrap();
        let mut unread: Vec<(String, u32)> = state
            .conversations
            .iter()
            .filter(|c| c.unread > 0)
            .map(|c| (state.conversation_title(c.id), c.unread))
            .collect();
        unread.sort();
        assert_eq!(
            unread,
            [
                ("Jim Halpert".to_owned(), 1),
                ("Scranton Branch 📄".to_owned(), 2)
            ],
            "{unread:?}"
        );
    }

    /// The poster and the live demo are one picture (WEBSITE.md §3.2): the
    /// session's own start, drawn with the poster's clock, gives the same
    /// buffer at the panel's size.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn the_poster_is_the_live_demo_start() {
        let (mut session, mut inbox) = live(&poster_mock()).await;
        settle(&mut session, &mut inbox, started).await;
        assert_eq!(
            session.state.open.as_ref().map(|open| open.id),
            chat(&session.state),
            "the session opened the branch chat itself"
        );
        let live = draw(&session.state, 120, 34);
        let poster = first_frame(120, 34).await.unwrap();
        assert_eq!(live.area, poster.area, "the same panel size");
        assert_eq!(
            faro_app::ui::text::grid(&live),
            faro_app::ui::text::grid(&poster),
            "the poster is the demo's first frame"
        );
    }

    /// The demo opens its first chat without the keyboard (D125): it must not
    /// claim focus, mark the backlog read, or send read receipts — the visitor
    /// has not given faro the keys yet.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn open_start_opens_the_chat_without_claiming_focus_or_marking_read() {
        let (mut state, store) = stored_state(&poster_mock()).await.unwrap();
        assert!(!state.env.focused, "the demo starts unfocused (D125)");
        let id = chat(&state).expect("the branch is in the list");
        let before = state
            .conversations
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.unread)
            .expect("the branch is listed");
        assert!(before > 0, "the backlog left it unread: {before}");

        let (chat_id, effects) = open_start(&mut state).expect("open_start opens the branch");
        assert_eq!(chat_id, id);
        assert!(!state.env.focused, "opening keeps the demo unfocused");
        assert_eq!(
            state.focus,
            Focus::Conversations,
            "the list keeps the focus"
        );
        let open = state.open.as_ref().expect("the branch is open");
        assert!(open.selected.is_none(), "nothing is selected (§4.2)");
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::MarkRead(_))),
            "no store write marking the chat read"
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Backend(Command::MarkRead(_)))),
            "no read receipts"
        );
        let after = state
            .conversations
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.unread)
            .expect("still listed");
        assert_eq!(after, before, "the unread count is kept until focus");
        store.close().await.unwrap();
    }

    /// What the 400-message backlog costs the live demo at start (D122): the
    /// mock, the memory store and the reducer, on the real clock, up to the
    /// settled first frame. The mock's own 400 ms connect delay is part of it.
    #[tokio::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn a_visitor_start_with_the_full_backlog_is_quick() {
        let began = std::time::Instant::now();
        let (mut session, mut inbox) = live(&visitor_mock()).await;
        settle(&mut session, &mut inbox, started).await;
        let took = began.elapsed();
        // The panel must be there within a second of the files arriving
        // (WEBSITE.md §6.6); the mock's connect delay is most of this.
        assert!(took < Duration::from_secs(3), "the start took {took:?}");
    }
}
