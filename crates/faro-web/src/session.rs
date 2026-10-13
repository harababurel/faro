//! A demo session (D114): the mock backend, the in-memory store, the state
//! and the two workers, wired the way the terminal app wires them. Nothing
//! here touches the DOM, so the whole pipeline is tested natively.

use std::path::Path;
use std::rc::Rc;

use faro_app::app::commands;
use faro_app::app::event::{AppEvent, Command, Effect};
use faro_app::app::palette::{
    ItemKind, Level, Preview, Setting, SettingChoice, items as palette_items,
};
use faro_app::app::state::State;
use faro_app::app::storage;
use faro_app::app::term_event::Event;
use faro_app::app::update::{self, affects_view};
use faro_app::theme::Theme;
use faro_core::backend::{Backend, BackendError, BackendEvent, BackendEvents};
use faro_core::domain::{AttachmentRef, AttachmentStatus, MessageRef, TranscriptSource};
use faro_core::mock::{MockBackend, MockConfig};
use faro_core::rt;
use faro_core::store::Store;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::blobs::Blobs;
use crate::demo::{self, Error};
use crate::router::{Page, Router};

/// The state and where its effects go.
pub struct Session {
    pub state: State,
    pub router: Router,
    /// The attachments downloaded so far.
    pub blobs: Blobs,
    /// Backlog messages still to come before the demo opens its first chat
    /// (D122): the mock sends them in one burst, and the poster shows the
    /// whole of them.
    waiting_backlog: usize,
    /// Whether the demo's opening chat has been opened (D122): once, as soon
    /// as its backlog has arrived.
    opened_start: bool,
    /// Whether the visitor has taken the demo in hand before its opening chat
    /// (D122): a key, click, wheel or paste of their own, a palette or popup
    /// they opened, or a chat they opened. Then the opening is never applied
    /// — it would drive whatever the visitor has open, not the list.
    visitor_acted: bool,
    /// The workers; stopped when the session ends.
    tasks: Vec<rt::Task>,
}

/// Whether `event` is the mock accepting a message the visitor sent (§6.9's
/// `Demo message sent` signal): only `Command::Send` results answer with
/// `SendResult`, so incoming messages, the backlog, reactions, edits,
/// deletes and downloads never count. A retried send answers with the same
/// event; the page counts one per visit. Only wasm's `live` and the tests
/// use it, so it's absent in native builds that aren't tests.
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn accepted_send(event: &AppEvent) -> bool {
    matches!(event, AppEvent::SendResult { result: Ok(()), .. })
}

/// What the app loop listens to besides the page.
pub struct Inbox {
    pub backend: BackendEvents,
    pub results: UnboundedReceiver<AppEvent>,
}

impl Session {
    /// Start the mock and the workers, and ask for the chat list, as the
    /// terminal app does on start.
    pub async fn start(mock: &MockConfig, page: Rc<dyn Page>) -> Result<(Self, Inbox), Error> {
        let (backend, backend_events) = MockBackend::new(mock);
        let me = backend.whoami().await?.service_id;
        let contacts = backend.contacts().await?;
        let groups = backend.groups().await?;
        let capabilities = backend.capabilities();
        let waiting_backlog = backend.backlog_len();
        let store = Store::memory();
        store.upsert_contacts(contacts.clone()).await?;
        store.upsert_groups(groups.clone()).await?;

        let mut state = State::new(me, contacts, groups);
        let blobs = Blobs::default();
        let (results_tx, results) = mpsc::unbounded_channel();
        let (effects, effect_queue) = mpsc::unbounded_channel();
        let (commands, command_queue) = mpsc::unbounded_channel();
        let tasks = vec![
            rt::spawn(store_worker(store, effect_queue, results_tx.clone())),
            rt::spawn(command_task(
                backend,
                blobs.clone(),
                command_queue,
                results_tx.clone(),
            )),
        ];
        let router = Router {
            commands,
            effects,
            results: results_tx,
            page,
            blobs: blobs.clone(),
        };
        // The demo's profile, environment and settings (D114, D116, D118),
        // shared with the site poster (`demo::configure_visitor`).
        let configured = demo::configure_visitor(&mut state, capabilities);
        router.dispatch(configured, &state);
        router.dispatch(vec![Effect::LoadConversations], &state);
        let session = Self {
            state,
            router,
            blobs,
            waiting_backlog,
            opened_start: false,
            visitor_acted: false,
            tasks,
        };
        let inbox = Inbox {
            backend: backend_events,
            results,
        };
        Ok((session, inbox))
    }

    /// One event through the reducer; whether it can change the picture.
    pub fn handle(&mut self, event: AppEvent) -> bool {
        if matches!(
            event,
            AppEvent::Terminal(Event::Key(_) | Event::Mouse(_) | Event::Paste(_))
        ) {
            self.visitor_acted = true;
        }
        if matches!(event, AppEvent::Backend(BackendEvent::Message(_))) {
            self.waiting_backlog = self.waiting_backlog.saturating_sub(1);
        }
        let redraw = affects_view(&event);
        let effects = update::update(&mut self.state, event);
        self.router.dispatch(effects, &self.state);
        self.open_start() || redraw
    }

    /// Whether the start has settled (D124): the chat list is in and the
    /// opening chat's history loaded — or the visitor took over first. The
    /// page swaps the poster for the canvas only then, so the swap shows the
    /// picture the poster already did.
    pub fn settled(&self) -> bool {
        self.opened_start
            && !self.state.loading
            && self.state.open.as_ref().is_none_or(|open| !open.loading)
    }

    /// The demo opens the branch chat once its whole backlog has arrived, so
    /// its first frame is the poster the page already shows (D122, WEBSITE.md
    /// §3.2). Before that the chat isn't in the list at all, and the store
    /// answers with an empty one, so this is tried after every event until
    /// the backlog is in and it takes.
    ///
    /// It is skipped for good once the visitor has done anything of their own
    /// (D122): the opening keys go through the reducer, so with a palette, a
    /// popup or another chat already open they would move its cursor and run
    /// its highlighted item.
    fn open_start(&mut self) -> bool {
        if self.opened_start || self.waiting_backlog > 0 || self.state.loading {
            return false;
        }
        if self.visitor_acted
            || self.state.open.is_some()
            || self.state.palette.is_some()
            || self.state.popup.is_some()
        {
            self.opened_start = true;
            return false;
        }
        let Some((_, effects)) = demo::open_start(&mut self.state) else {
            return false;
        };
        self.opened_start = true;
        self.router.dispatch(effects, &self.state);
        true
    }

    /// The page's theme chip (WEBSITE.md §4.5): the palette's own path —
    /// `choose_theme` sets the theme and notices it; its save effect is
    /// dropped by the router, so the choice lasts for the visit (§4.8).
    /// `false` if there's no such theme. While the Themes palette is open
    /// its stale Theme preview is deliberately ended (the chosen theme
    /// must draw, not the row the cursor sits on) and the theme rows'
    /// radio marks move onto the chosen one; the list, query and path
    /// stay as they are, and stepping the cursor previews again from
    /// there. A preview of a setting is none of the chip's business.
    pub fn pick_theme(&mut self, name: &str) -> bool {
        if Theme::find(name, &self.state.user_themes).is_none() {
            return false;
        }
        self.visitor_acted = true;
        let effects = update::choose_theme(&mut self.state, name);
        if let Some(palette) = self.state.palette.as_mut() {
            if matches!(palette.preview, Some(Preview::Theme(_))) {
                palette.preview = None;
            }
            for item in &mut palette.items {
                if let ItemKind::Theme(row) = &item.kind {
                    item.current = Some(row == name);
                }
            }
        }
        self.router.dispatch(effects, &self.state);
        true
    }

    /// The page's motion toggle (§4.6): faro's own `reduce_motion` setting
    /// through the shared settings path (notice and visit-only save). It
    /// stills the lighthouse (`State::logo_moves`) and touches nothing else.
    pub fn set_reduce_motion(&mut self, reduced: bool) {
        self.visitor_acted = true;
        let effects = update::choose(&mut self.state, SettingChoice::ReduceMotion(reduced));
        self.refresh_motion_rows();
        self.router.dispatch(effects, &self.state);
    }

    /// The page's motion preference before the handover (§4.6): the same
    /// shared path, but quietly — the notice is put back as it was (the
    /// poster's picture must not gain one) and it is not a visitor
    /// takeover, so the demo's own opening chat still opens.
    pub fn configure_reduce_motion(&mut self, reduced: bool) {
        let previous = self.state.notice.take();
        let effects = update::choose(&mut self.state, SettingChoice::ReduceMotion(reduced));
        self.state.notice = previous;
        self.refresh_motion_rows();
        self.router.dispatch(effects, &self.state);
    }

    /// After a motion change from outside the palette, refresh the open
    /// palette's rows that show that setting, so a row doesn't sit on the
    /// old value while the page's toolbar shows the new one — the shared
    /// palette `flip` does this for changes made inside the palette. Only
    /// the motion rows are replaced, with the shared builder's fresh rows
    /// for the open level (the settings list and the setting's own
    /// values); positions, path, query, scroll and selection stay put —
    /// titles don't change, so the query still matches the same rows —
    /// and previews and other levels are none of this change's business.
    fn refresh_motion_rows(&mut self) {
        let Some(palette) = self.state.palette.as_ref() else {
            return;
        };
        let level = palette.level();
        if !matches!(
            level,
            Level::Settings | Level::Setting(Setting::ReduceMotion)
        ) {
            return;
        }
        let fresh = palette_items(&self.state, level, &self.state.recent_commands);
        let Some(palette) = self.state.palette.as_mut() else {
            return;
        };
        for item in &mut palette.items {
            if !matches!(
                item.kind,
                ItemKind::Switch(Setting::ReduceMotion)
                    | ItemKind::Setting(SettingChoice::ReduceMotion(_))
            ) {
                continue;
            }
            if let Some(replacement) = fresh.iter().find(|row| row.kind == item.kind) {
                *item = replacement.clone();
            }
        }
    }

    /// The "all…" chip: the palette at Themes, through the shared opener —
    /// no synthetic keys (D125), so the keys, the focused pane and any
    /// draft stay as they are.
    pub fn open_themes(&mut self) {
        self.visitor_acted = true;
        update::open_palette(&mut self.state, Level::Themes);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Store effects one at a time, in order, as in the terminal app.
async fn store_worker(
    store: Store,
    mut queue: UnboundedReceiver<Effect>,
    results: UnboundedSender<AppEvent>,
) {
    while let Some(effect) = queue.recv().await {
        if let Some(event) = storage::execute(&store, effect).await
            && results.send(event).is_err()
        {
            tracing::debug!("app loop stopped before a store result arrived");
        }
    }
}

/// Backend commands one at a time, in order, as in the terminal app.
async fn command_task(
    backend: MockBackend,
    blobs: Blobs,
    mut queue: UnboundedReceiver<Command>,
    results: UnboundedSender<AppEvent>,
) {
    while let Some(command) = queue.recv().await {
        let event = match command {
            Command::Fetch {
                conversation,
                message,
                index,
                attachment,
            } => Some(AppEvent::Fetched {
                conversation,
                message,
                index,
                result: fetch(&backend, &blobs, message, index, attachment).await,
            }),
            Command::Transcribe {
                conversation,
                message,
                index,
                path,
                ..
            } => Some(AppEvent::Transcribed {
                conversation,
                message,
                index,
                result: canned_transcript(&blobs, &path),
            }),
            command @ (Command::Send { .. }
            | Command::React { .. }
            | Command::Edit { .. }
            | Command::DeleteForEveryone { .. }
            | Command::MarkRead(_)
            | Command::Typing { .. }) => commands::execute(&backend, command).await,
        };
        if let Some(event) = event
            && results.send(event).is_err()
        {
            tracing::debug!("app loop stopped before a backend result arrived");
        }
    }
}

/// Download an attachment into memory (the terminal app writes a file).
async fn fetch(
    backend: &MockBackend,
    blobs: &Blobs,
    message: MessageRef,
    index: usize,
    mut attachment: AttachmentRef,
) -> Result<AttachmentRef, BackendError> {
    let bytes = backend.fetch_attachment(&attachment).await?;
    if attachment.is_voice_note()
        && (attachment.duration_secs.is_none() || attachment.waveform.is_none())
        && let Some(info) = faro_core::audio::inspect_wav(&bytes)
    {
        attachment.duration_secs = attachment.duration_secs.or(info.duration_secs);
        attachment.waveform = attachment.waveform.or(Some(info.waveform));
    }
    attachment.size = u64::try_from(bytes.len()).ok();
    let path = blobs.insert(message, index, attachment.id.clone(), bytes);
    attachment.status = AttachmentStatus::Downloaded(path);
    Ok(attachment)
}

/// What the demo's voice notes say (WEBSITE.md §4.8): no speech
/// recognition, no network, and marked `[demo]` on screen.
const TRANSCRIPTS: &[(&str, &str)] = &[
    (
        "mock:voice-michael",
        "Hey, it's Michael. Call me back. It's urgent. Not urgent-urgent, but urgent.",
    ),
    (
        "mock:voice-andy",
        "Rit-dit-dit-doo, doo-doo, rit-dit-dit-doo… the Here Comes Treble remix.",
    ),
];

fn canned_transcript(blobs: &Blobs, path: &Path) -> Result<(String, TranscriptSource), String> {
    let blob = blobs
        .get(path)
        .ok_or_else(|| "the voice note isn't downloaded".to_owned())?;
    TRANSCRIPTS
        .iter()
        .find(|(id, _)| *id == blob.id.0)
        .map(|(_, text)| ((*text).to_owned(), TranscriptSource::Demo))
        .ok_or_else(|| "no transcript for this voice note in the demo".to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use faro_app::app::palette::Item;
    use faro_app::app::state::{EmojiMode, Focus, Geometry, Notice};
    use faro_app::app::term_event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use faro_core::domain::{ConnectionState, Direction};

    use super::*;
    use crate::demo::visitor_mock;
    use crate::router::NOT_HERE;
    use crate::router::tests::FakePage;

    async fn started() -> (Session, Inbox, Rc<FakePage>) {
        let page = Rc::new(FakePage::default());
        let shared: Rc<dyn Page> = page.clone();
        let (mut session, mut inbox) = Session::start(&visitor_mock(), shared).await.unwrap();
        assert!(!session.settled(), "nothing has arrived yet");
        pump(&mut session, &mut inbox, Duration::from_secs(1)).await;
        (session, inbox, page)
    }

    /// Feed everything that arrives within `time` to the session.
    async fn pump(session: &mut Session, inbox: &mut Inbox, time: Duration) {
        let until = rt::Instant::now() + time;
        loop {
            let event = tokio::select! {
                Some(event) = inbox.backend.recv() => AppEvent::Backend(event),
                Some(event) = inbox.results.recv() => event,
                () = rt::sleep_until(until) => return,
            };
            session.handle(event);
        }
    }

    fn key(session: &mut Session, code: KeyCode) {
        session.handle(AppEvent::Terminal(Event::Key(KeyEvent::from(code))));
    }

    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn a_visitor_can_open_a_chat_and_send_a_message() {
        let (mut session, mut inbox, _page) = started().await;
        let state = &session.state;
        assert!(!state.loading, "the chat list loaded");
        assert_eq!(state.connection, ConnectionState::Connected);
        assert!(
            state.conversations.len() > 3,
            "{}",
            state.conversations.len()
        );
        // The demo opens the branch chat once its backlog is in, so its first
        // frame is the poster the page already shows (D122).
        let open = state.open.as_ref().expect("the branch chat opens");
        assert_eq!(Some(open.id), demo::chat(state), "the branch chat");
        assert!(!open.loading && !open.messages.is_empty(), "history loaded");
        assert_eq!(state.focus, Focus::Conversations, "the list has the focus");
        assert!(open.selected.is_none(), "nothing is selected (§4.2)");
        assert!(session.settled(), "the page may swap the poster out");
        assert_eq!(
            state.env.emoji.mode,
            EmojiMode::Exact,
            "after the terminal check"
        );

        key(&mut session, KeyCode::Enter);
        for c in "hello".chars() {
            key(&mut session, KeyCode::Char(c));
        }
        key(&mut session, KeyCode::Enter);
        pump(&mut session, &mut inbox, Duration::from_millis(100)).await;
        let open = session.state.open.as_ref().unwrap();
        let sent = open
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.direction, Direction::Outgoing(_)))
            .unwrap();
        assert_eq!(
            sent.body
                .as_ref()
                .map(faro_core::domain::MessageBody::expose),
            Some("hello")
        );
        assert!(session.state.notice.is_none(), "{:?}", session.state.notice);
    }

    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn the_page_copies_and_files_get_a_notice() {
        let (mut session, mut inbox, page) = started().await;
        key(&mut session, KeyCode::Enter);
        pump(&mut session, &mut inbox, Duration::from_millis(100)).await;

        // A path pasted into the input is just text.
        let paste = Event::Paste("/etc/hosts".to_owned());
        session.handle(AppEvent::Terminal(paste));
        pump(&mut session, &mut inbox, Duration::from_millis(100)).await;
        let open = session.state.open.as_ref().unwrap();
        assert_eq!(open.input.text(), "/etc/hosts");
        assert!(open.staged_attachment.is_none());

        // Copying goes to the page.
        session
            .router
            .dispatch(vec![Effect::Copy("hi".to_owned())], &session.state);
        assert_eq!(
            page.log.borrow().last().map(String::as_str),
            Some("copy hi")
        );

        // Files aren't here.
        session
            .router
            .dispatch(vec![Effect::ListFiles("~".to_owned())], &session.state);
        pump(&mut session, &mut inbox, Duration::from_millis(100)).await;
        let notice = session.state.notice.as_ref().unwrap();
        assert_eq!(notice.text, NOT_HERE);
    }

    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn the_demo_never_opens_a_chat_over_the_visitor_s_own_start() {
        let page = Rc::new(FakePage::default());
        let shared: Rc<dyn Page> = page.clone();
        let (mut session, mut inbox) = Session::start(&visitor_mock(), shared).await.unwrap();
        assert!(
            session.waiting_backlog > 0,
            "the backlog is still to come: {} messages",
            session.waiting_backlog
        );

        // The visitor opens the palette before the demo's opening chat.
        let ctrl_p = Event::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        session.handle(AppEvent::Terminal(ctrl_p));
        pump(&mut session, &mut inbox, Duration::from_secs(1)).await;

        // The opening keys would have driven the palette instead of the list:
        // Alt+Down its cursor, Enter the highlighted item. None of that
        // happened, and no chat was opened.
        let palette = session
            .state
            .palette
            .as_ref()
            .expect("the palette stays open");
        assert_eq!(
            palette.path.last(),
            Some(&Level::Root),
            "{:?}",
            palette.path
        );
        assert_eq!(palette.selected, 0, "no item was chosen");
        assert!(palette.input.text().is_empty(), "the query is untouched");
        assert!(session.state.open.is_none(), "no chat was opened");
        assert!(
            session.settled(),
            "the visitor's own start counts as settled"
        );
    }

    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn voice_notes_download_into_memory_play_and_have_transcripts() {
        let (mut session, mut inbox, page) = started().await;
        let message = MessageRef {
            sender: session.state.me,
            sent_at: faro_core::domain::Timestamp::from_millis(1),
        };
        let note = AttachmentRef::remote("mock:voice-andy", "audio/wav");
        let (backend, _events) = MockBackend::new(&visitor_mock());
        // It connects first.
        rt::sleep(Duration::from_secs(5)).await;
        let fetched = fetch(&backend, &session.blobs, message, 0, note)
            .await
            .unwrap();
        let AttachmentStatus::Downloaded(path) = &fetched.status else {
            panic!("{:?}", fetched.status);
        };
        assert!(fetched.size.unwrap() > 1000);

        let (text, source) = canned_transcript(&session.blobs, path).unwrap();
        assert_eq!(source, TranscriptSource::Demo);
        assert!(text.contains("Treble"), "{text}");
        assert!(canned_transcript(&session.blobs, Path::new("/nope")).is_err());

        let play = Effect::PlayAudio {
            message,
            index: 0,
            path: path.clone(),
            player: String::new(),
        };
        session.router.dispatch(vec![play], &session.state);
        pump(&mut session, &mut inbox, Duration::from_millis(10)).await;
        let played = format!("play {}", fetched.size.unwrap());
        assert!(
            page.log.borrow().contains(&played),
            "{:?}",
            page.log.borrow()
        );

        // Copying its path: the path means nothing outside the demo.
        let copy = Effect::Copy(path.display().to_string());
        session.router.dispatch(vec![copy], &session.state);
        pump(&mut session, &mut inbox, Duration::from_millis(10)).await;
        assert_eq!(session.state.notice.as_ref().unwrap().text, NOT_HERE);
    }

    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn escape_releases_focus_only_at_the_end_of_the_chain() {
        let (mut session, _inbox, page) = started().await;
        // The demo opened the branch chat with the list focused (D60).
        assert!(session.state.open.is_some());
        session.state.notice = None; // ignore any notice the backlog left
        key(&mut session, KeyCode::Enter); // focus the input
        assert_eq!(session.state.focus, Focus::Input);

        let esc = |session: &mut Session| {
            session.handle(AppEvent::Terminal(Event::Key(KeyEvent::from(KeyCode::Esc))));
        };
        let released = || page.log.borrow().iter().any(|l| l == "release_focus");

        // Closing the open chat is a step, not a release.
        esc(&mut session);
        assert!(session.state.open.is_none(), "the chat closed");
        assert!(
            !released(),
            "no release while a chat was still open to close"
        );

        // With nothing open and no notice, the page blurs the canvas.
        esc(&mut session);
        assert!(released(), "the end of the chain releases focus");
    }

    /// The predicate behind the `faro-sent` signal (§6.9): only a
    /// successful send result counts; everything else the loop can see
    /// doesn't, so incoming messages, backlog, reactions, edits, deletes,
    /// failed sends and downloads never fire the analytics event.
    #[test]
    fn accepted_send_counts_only_successful_send_results() {
        let sent_at = faro_core::domain::Timestamp::from_millis(1);
        let who: faro_core::domain::ServiceId =
            "00000000-0000-0000-0000-00000000000a".parse().unwrap();
        let target = MessageRef {
            sender: who,
            sent_at,
        };
        let cases = [
            (
                AppEvent::SendResult {
                    sent_at,
                    result: Ok(()),
                },
                true,
            ),
            (
                AppEvent::SendResult {
                    sent_at,
                    result: Err(faro_core::backend::BackendError::Network("down".to_owned())),
                },
                false,
            ),
            (
                AppEvent::ReactResult {
                    conversation: who.into(),
                    target,
                    previous: None,
                    result: Ok(()),
                },
                false,
            ),
            (AppEvent::EditResult(Ok(())), false),
            (AppEvent::DeleteResult(Ok(())), false),
            (AppEvent::Tick, false),
        ];
        for (event, accepted) in cases {
            assert_eq!(accepted_send(&event), accepted, "{event:?}");
        }
    }

    /// The demo opens its first chat without the keys (D125): it must not send
    /// read receipts or clear the unread count until the browser hands faro
    /// the keys with a real `FocusGained`.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn an_unfocused_start_holds_read_receipts_until_the_keys_arrive() {
        let (mut session, mut inbox, _page) = started().await;
        assert!(!session.state.env.focused, "the demo starts unfocused");
        let open = session.state.open.as_ref().expect("the branch is open");
        assert_eq!(Some(open.id), demo::chat(&session.state));
        // Opening while unfocused keeps the unread count (D125).
        let branch = session
            .state
            .conversations
            .iter()
            .find(|c| c.id == open.id)
            .expect("the branch is listed");
        assert!(
            branch.unread > 0,
            "the unread count is kept: {}",
            branch.unread
        );
        // The incoming messages are queued for a receipt, not sent.
        assert!(!open.unreceipted.is_empty(), "receipts wait for the keys");

        // A frame drawn while unfocused sends no receipt and clears nothing.
        let effects = update::update(&mut session.state, AppEvent::Drawn(Geometry::default()));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Backend(Command::MarkRead(_)))),
            "no read receipts while unfocused"
        );
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::MarkRead(_))),
            "no unread clear while unfocused"
        );

        // The visitor gives faro the keys: the receipts go out.
        let effects = update::update(&mut session.state, AppEvent::Terminal(Event::FocusGained));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Backend(Command::MarkRead(_)))),
            "focus sends the read receipts"
        );
        assert!(session.state.env.focused, "the keys are faro's now");
        pump(&mut session, &mut inbox, Duration::from_millis(100)).await;
    }

    /// A theme chip goes through the palette's own path: the setting, the
    /// notice, and a save the web router drops (settings last for the visit,
    /// D-c). It never takes the keys (D125).
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn a_theme_chip_switches_like_the_palette_does() {
        let (mut session, mut inbox, page) = started().await;
        assert!(
            !session.pick_theme("no-such-theme"),
            "an unknown theme is refused"
        );
        assert_eq!(session.state.config.theme, "faro-dark", "left alone");
        assert!(session.pick_theme("nord"));
        assert_eq!(session.state.config.theme, "nord", "the setting");
        assert_eq!(session.state.theme_name(), "nord", "and what draws");
        assert_eq!(
            session.state.notice.as_ref().expect("a notice").text,
            "Theme: nord",
            "the palette's own notice"
        );
        assert!(session.visitor_acted, "a chip is the visitor taking over");
        assert!(!session.state.env.focused, "the chip doesn't take the keys");
        pump(&mut session, &mut inbox, Duration::from_millis(100)).await;
        assert!(
            page.log.borrow().is_empty(),
            "the dropped save reaches neither page nor store: {:?}",
            page.log.borrow()
        );
    }

    /// A chip while the Themes palette previews another row must show the
    /// chosen theme, not the stale preview.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn a_chip_under_a_theme_preview_shows_the_chosen_theme() {
        let (mut session, _inbox, _page) = started().await;
        session.open_themes();
        let palette = session
            .state
            .palette
            .as_ref()
            .expect("the palette is at Themes");
        assert_eq!(palette.path.last(), Some(&Level::Themes));
        // What stepping the cursor onto another row does (D54).
        session.state.palette.as_mut().expect("still open").preview =
            Some(Preview::Theme("tokyonight".to_owned()));
        assert_eq!(
            session.state.theme_name(),
            "tokyonight",
            "the preview draws"
        );
        assert!(session.pick_theme("nord"));
        assert_eq!(
            session.state.theme_name(),
            "nord",
            "the chip wins over the stale preview"
        );
        assert!(session.state.palette.is_some(), "the palette stays open");
    }

    /// A chip while the Themes palette is open moves the radio mark onto
    /// the chosen row and only that row, and keeps the query, the path
    /// and the keys exactly as they were.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn a_chip_under_the_themes_palette_marks_the_chosen_row_only() {
        let (mut session, _inbox, _page) = started().await;
        session.open_themes();
        let palette = session
            .state
            .palette
            .as_mut()
            .expect("the palette is at Themes");
        palette.input.insert_str("o"); // a query matching several themes
        palette.preview = Some(Preview::Theme("tokyonight".to_owned()));
        assert!(session.pick_theme("nord"));
        let palette = session.state.palette.as_ref().expect("still open");
        let themes: Vec<(&str, Option<bool>)> = palette
            .items
            .iter()
            .filter_map(|item| {
                if let ItemKind::Theme(name) = &item.kind {
                    Some((name.as_str(), item.current))
                } else {
                    None
                }
            })
            .collect();
        assert!(!themes.is_empty(), "the Themes level lists themes");
        let marked: Vec<&str> = themes
            .iter()
            .filter(|(_, current)| *current == Some(true))
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(marked, ["nord"], "only the chosen row is marked");
        assert_eq!(palette.input.text(), "o", "the query is kept");
        assert_eq!(
            palette.path.last(),
            Some(&Level::Themes),
            "the path is kept"
        );
        assert!(palette.preview.is_none(), "the stale Theme preview ended");
        assert!(!session.state.env.focused, "the keys stay the page's");
    }

    /// The motion toggle goes through faro's own setting and stills the
    /// lighthouse (D61); it doesn't touch the keys.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn the_motion_toggle_stills_the_lighthouse() {
        let (mut session, _inbox, _page) = started().await;
        // The logo only moves with nothing open and the keys (D61): set that up.
        session.state.open = None;
        session.state.env.focused = true;
        assert!(session.state.logo_moves(), "the beam turns by default");
        session.set_reduce_motion(true);
        assert!(session.state.config.reduce_motion, "faro's own setting");
        assert!(!session.state.logo_moves(), "reduced motion stills it");
        assert_eq!(
            session.state.notice.as_ref().expect("a notice").text,
            "Reduce motion: the lighthouse stands still"
        );
        session.set_reduce_motion(false);
        assert!(session.state.logo_moves(), "motion comes back");
        assert!(
            session.state.env.focused,
            "the toggle doesn't touch the keys"
        );
    }

    /// The page's motion preference at start is quiet: the shared path is
    /// still taken, but no notice lands on the poster's picture, a notice
    /// already there is kept, and it isn't a visitor takeover — the demo's
    /// own opening chat still opens (D125).
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn configuring_motion_at_start_is_quiet_and_keeps_the_opening() {
        let page = Rc::new(FakePage::default());
        let shared: Rc<dyn Page> = page.clone();
        let (mut session, mut inbox) = Session::start(&visitor_mock(), shared).await.unwrap();
        assert!(
            session.waiting_backlog > 0,
            "the backlog is still to come: {} messages",
            session.waiting_backlog
        );
        session.state.notice = Some(Notice::info("keep me".to_owned()));
        session.configure_reduce_motion(true);
        assert!(session.state.config.reduce_motion, "the setting took");
        assert_eq!(
            session.state.notice.as_ref().expect("the old notice").text,
            "keep me",
            "no notice over the poster, and the old one stays"
        );
        assert!(!session.visitor_acted, "not a visitor takeover");
        assert!(!session.state.env.focused, "the keys stay the page's");
        session.state.notice = None;
        pump(&mut session, &mut inbox, Duration::from_secs(1)).await;
        assert!(session.settled(), "the demo still opens its own chat");
        assert_eq!(
            session.state.open.as_ref().map(|open| open.id),
            demo::chat(&session.state),
            "the branch opened on its own"
        );
        assert!(
            session.state.config.reduce_motion,
            "and the motion preference stuck"
        );
    }

    /// A motion toggle from the page refreshes the rows of an open
    /// settings palette, so a row doesn't sit on the old value while the
    /// toolbar shows the new one (the shared palette `flip` does this for
    /// changes made inside the palette). Path, query, cursor, keys and
    /// draft stay put, and the refreshed row equals what the shared
    /// builder makes for the new config.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn a_motion_toggle_refreshes_an_open_settings_palette() {
        let (mut session, _inbox, _page) = started().await;
        key(&mut session, KeyCode::Enter); // focus the input
        for c in "keep me".chars() {
            key(&mut session, KeyCode::Char(c));
        }
        update::open_palette(&mut session.state, Level::Settings);
        let palette = session
            .state
            .palette
            .as_mut()
            .expect("the palette is at Settings");
        palette.input.insert_str("motion");
        let path = palette.path.clone();
        let selected = palette.selected;
        let before = motion_row(&session.state).expect("the row is listed");
        assert_eq!(before.detail, "off", "the demo's default");
        assert!(!before.changed, "off is the default, so unmarked");
        let focused = session.state.env.focused;
        session.set_reduce_motion(true);
        let palette = session.state.palette.as_ref().expect("still open");
        assert_eq!(palette.path, path, "the path is kept");
        assert_eq!(palette.input.text(), "motion", "the query is kept");
        assert_eq!(palette.selected, selected, "the cursor stays put");
        let after = motion_row(&session.state).expect("the row is still listed");
        assert_eq!(after.detail, "on", "the row shows the new value");
        assert!(after.changed, "on differs from the default");
        let fresh = palette_items(
            &session.state,
            Level::Settings,
            &session.state.recent_commands,
        );
        let expected = fresh
            .iter()
            .find(|row| row.kind == after.kind)
            .expect("the builder lists it too");
        assert_eq!(after, expected, "the row matches the shared builder");
        assert_eq!(session.state.env.focused, focused, "the keys are untouched");
        let open = session.state.open.as_ref().expect("the chat stays open");
        assert_eq!(open.input.text(), "keep me", "the draft stays");
        // The values level: the radio mark moves too, and the quiet
        // system update refreshes it the same way.
        update::open_palette(&mut session.state, Level::Setting(Setting::ReduceMotion));
        let marked = |session: &Session| {
            session
                .state
                .palette
                .as_ref()
                .expect("open")
                .items
                .iter()
                .find_map(|item| {
                    if let ItemKind::Setting(SettingChoice::ReduceMotion(on)) = &item.kind {
                        (item.current == Some(true)).then_some(*on)
                    } else {
                        None
                    }
                })
        };
        assert_eq!(marked(&session), Some(true), "the radio marks on");
        session.configure_reduce_motion(false);
        assert_eq!(
            marked(&session),
            Some(false),
            "the quiet update refreshes it too"
        );
    }

    /// The settings list's `Reduce motion` switch row.
    fn motion_row(state: &State) -> Option<&Item> {
        state
            .palette
            .as_ref()?
            .items
            .iter()
            .find(|item| matches!(item.kind, ItemKind::Switch(Setting::ReduceMotion)))
    }

    /// The "all…" chip opens the palette at Themes without synthetic keys
    /// (D125): the keys, the focused pane and the draft stay as they are.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn open_themes_keeps_the_keys_and_the_draft() {
        let (mut session, _inbox, _page) = started().await;
        key(&mut session, KeyCode::Enter); // focus the input
        for c in "keep me".chars() {
            key(&mut session, KeyCode::Char(c));
        }
        let focused = session.state.env.focused;
        session.open_themes();
        let palette = session.state.palette.as_ref().expect("the palette is open");
        assert_eq!(palette.path.last(), Some(&Level::Themes));
        assert!(
            palette.input.text().is_empty(),
            "no fake keys typed into the query"
        );
        assert_eq!(session.state.env.focused, focused, "the keys are untouched");
        let open = session.state.open.as_ref().expect("the chat stays open");
        assert_eq!(open.input.text(), "keep me", "the draft stays");
        assert!(session.visitor_acted, "the chip is the visitor taking over");
    }
}
