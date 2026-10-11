//! A demo session (D114): the mock backend, the in-memory store, the state
//! and the two workers, wired the way the terminal app wires them. Nothing
//! here touches the DOM, so the whole pipeline is tested natively.

use std::rc::Rc;

use faro_app::app::commands;
use faro_app::app::event::{AppEvent, Command, Effect};
use faro_app::app::state::{BlockGlyphs, GraphicsSupport, State};
use faro_app::app::storage;
use faro_app::app::update::{self, affects_view};
use faro_app::theme::ColorDepth;
use faro_core::backend::{Backend, BackendError, BackendEvents};
use faro_core::config::Config;
use faro_core::mock::{MockBackend, MockConfig};
use faro_core::rt;
use faro_core::store::{Profile, Store};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::demo::{Error, SEED};
use crate::router::{Page, Router};

/// The mock as visitors get it (D-b): the demo's seed, today's dates, the
/// default activity rate, no scripted outages or errors.
pub fn visitor_mock() -> MockConfig {
    MockConfig {
        seed: SEED,
        disconnects: false,
        errors: false,
        ..MockConfig::default()
    }
}

/// The state and where its effects go.
pub struct Session {
    pub state: State,
    pub router: Router,
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
        let store = Store::memory();
        store.upsert_contacts(contacts.clone()).await?;
        store.upsert_groups(groups.clone()).await?;

        let mut state = State::new(me, contacts, groups);
        state.profile = Profile::Demo;
        state.capabilities = backend.capabilities();
        state.env.glyphs = BlockGlyphs::Native;
        state.env.graphics = GraphicsSupport::HalfblocksOnly;
        state.env.colors = ColorDepth::Truecolor;

        let (results_tx, results) = mpsc::unbounded_channel();
        let (effects, effect_queue) = mpsc::unbounded_channel();
        let (commands, command_queue) = mpsc::unbounded_channel();
        let tasks = vec![
            rt::spawn(store_worker(store, effect_queue, results_tx.clone())),
            rt::spawn(command_task(backend, command_queue, results_tx.clone())),
        ];
        let router = Router {
            commands,
            effects,
            results: results_tx,
            page,
        };
        let configured = update::configure(&mut state, Config::default(), Vec::new());
        router.dispatch(configured, &state);
        router.dispatch(vec![Effect::LoadConversations], &state);
        let session = Self {
            state,
            router,
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
        let redraw = affects_view(&event);
        let effects = update::update(&mut self.state, event);
        self.router.dispatch(effects, &self.state);
        redraw
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
    mut queue: UnboundedReceiver<Command>,
    results: UnboundedSender<AppEvent>,
) {
    while let Some(command) = queue.recv().await {
        let event = match command {
            Command::Fetch {
                conversation,
                message,
                index,
                ..
            } => Some(AppEvent::Fetched {
                conversation,
                message,
                index,
                result: Err(BackendError::Storage("not in the web demo yet".to_owned())),
            }),
            Command::Transcribe {
                conversation,
                message,
                index,
                ..
            } => Some(AppEvent::Transcribed {
                conversation,
                message,
                index,
                result: Err("not in the web demo yet".to_owned()),
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use faro_app::app::term_event::{Event, KeyCode, KeyEvent};
    use faro_core::domain::{ConnectionState, Direction};

    use super::*;
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
        assert!(state.open.is_none(), "nothing opens by itself (D60)");

        key(&mut session, KeyCode::Enter);
        pump(&mut session, &mut inbox, Duration::from_millis(100)).await;
        let open = session.state.open.as_ref().unwrap();
        assert!(!open.loading && !open.messages.is_empty(), "history loaded");

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
}
