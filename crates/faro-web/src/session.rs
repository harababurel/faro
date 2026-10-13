//! A demo session (D114): the mock backend, the in-memory store, the state
//! and the two workers, wired the way the terminal app wires them. Nothing
//! here touches the DOM, so the whole pipeline is tested natively.

use std::path::Path;
use std::rc::Rc;

use faro_app::app::commands;
use faro_app::app::event::{AppEvent, Command, Effect};
use faro_app::app::state::State;
use faro_app::app::storage;
use faro_app::app::term_event::Event;
use faro_app::app::update::{self, affects_view};
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
    /// they opened, or a chat they opened. Then the opening keys are never
    /// injected — they would drive whatever the visitor has open, not the list.
    visitor_acted: bool,
    /// The workers; stopped when the session ends.
    tasks: Vec<rt::Task>,
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

    use faro_app::app::palette::Level;
    use faro_app::app::state::{EmojiMode, Focus};
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
}
