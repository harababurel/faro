//! What flows into the reducer (`AppEvent`) and out of it (`Effect`), D22.

use std::path::PathBuf;

use faro_core::backend::{
    BackendError, BackendEvent, OutgoingEdit, OutgoingMessage, ReactionChange,
};
use faro_core::clipboard::ReadError;
use faro_core::config::{SettingEdit, Voice};
use faro_core::domain::{
    AttachmentRef, ConversationId, Message, MessageRef, ServiceId, Timestamp, TranscriptSource,
    TypingState,
};
use faro_core::store::{ConversationState, ConversationSummary, RecipientReceipt, SearchHit};

use super::state::{Geometry, Notice};
use super::updates::Release;
use crate::theme::ThemeText;

#[derive(Debug)]
pub enum AppEvent {
    Terminal(crate::app::term_event::Event),
    Backend(BackendEvent),
    /// Startup data from the store.
    Loaded {
        conversations: Vec<ConversationSummary>,
        last_open: Option<ConversationId>,
        /// The reactions last sent from here, most recent first, as stored
        /// (D104).
        recent_reactions: Vec<String>,
    },
    MessagesLoaded {
        conversation: ConversationId,
        messages: Vec<Message>,
    },
    /// A page of messages older than the first one loaded.
    OlderLoaded {
        conversation: ConversationId,
        messages: Vec<Message>,
    },
    /// A page of messages around a target message (`Effect::LoadMessagesAround`).
    MessagesAroundLoaded {
        conversation: ConversationId,
        target: MessageRef,
        messages: Vec<Message>,
    },
    /// Search hits across message bodies and transcripts (`Effect::SearchMessages`).
    SearchResults {
        conversation: Option<ConversationId>,
        query: String,
        hits: Vec<SearchHit>,
    },
    /// Per-recipient receipts loaded for `Popup::MessageInfo` (M12).
    MessageReceiptsLoaded {
        conversation: ConversationId,
        message: MessageRef,
        receipts: Vec<RecipientReceipt>,
    },
    /// The backend finished (or gave up) sending our message `sent_at`.
    SendResult {
        sent_at: Timestamp,
        result: Result<(), BackendError>,
    },
    /// The backend finished (or gave up) sending our reaction; on failure,
    /// `previous` is restored.
    ReactResult {
        conversation: ConversationId,
        target: MessageRef,
        previous: Option<String>,
        result: Result<(), BackendError>,
    },
    /// The backend finished (or gave up) editing our message.
    EditResult(Result<(), BackendError>),
    /// The backend finished (or gave up) deleting our message for everyone.
    DeleteResult(Result<(), BackendError>),
    /// An attachment finished downloading (or failed) (D75).
    Fetched {
        conversation: ConversationId,
        message: MessageRef,
        index: usize,
        result: Result<AttachmentRef, BackendError>,
    },
    /// A voice-note transcription finished (`Command::Transcribe`, D82).
    Transcribed {
        conversation: ConversationId,
        message: MessageRef,
        index: usize,
        result: Result<(String, TranscriptSource), String>,
    },
    /// Copying a downloaded attachment to `~/Downloads/` finished (D75).
    SavedToDownloads(Result<PathBuf, String>),
    /// Whether the local transcription command is installed (D145).
    CommandChecked {
        program: String,
        found: bool,
    },
    /// Directory entries read for the palette's file picker (`Level::AttachFile`, D77).
    FilesListed {
        query: String,
        entries: Vec<super::state::FileEntry>,
    },
    /// A file chosen in the palette was inspected and staged (D77).
    FileStaged(Result<super::state::StagedAttachment, String>),
    /// A pasted path candidate was checked on disk (D77). `None` means it is
    /// not a regular file, so `raw` should be pasted as text.
    PasteChecked {
        raw: String,
        staged: Option<super::state::StagedAttachment>,
    },
    /// A clipboard image read (`Ctrl+V`, D91) is over: the image to stage in
    /// `conversation`, or why not. The app loop makes a new `EventStream`
    /// first (input was paused for the read, D90).
    ClipboardImage {
        conversation: ConversationId,
        result: Result<super::state::StagedAttachment, ClipboardError>,
    },
    /// The clipboard has an image and the terminal is about to ask the user
    /// for permission: say whose prompt to answer (D150).
    ClipboardPrompt(super::state::TerminalName),
    /// A late clipboard reply was drained (D90); input resumes.
    ClipboardDrained,
    /// External audio playback finished or failed (D81).
    PlaybackFinished {
        message: MessageRef,
        index: usize,
        error: Option<String>,
    },
    /// Opening a link or attachment failed; the text is safe to show.
    OpenFailed(String),
    /// The platform can't do what an effect asked for (the web demo has no
    /// files or processes, D115); the text is safe to show.
    Unavailable(String),
    /// What's between faro and the user: mosh (D47), and who draws block
    /// glyphs (D58).
    TerminalChecked(super::state::Terminal),
    /// The config file changed and was read again (D48); the error is safe
    /// to show.
    ConfigRead(Result<String, String>),
    /// Writing a setting to the config file failed; the text is safe to show.
    SettingNotSaved(String),
    /// A file in the user's `themes/` directory changed; every theme file,
    /// read again (D54).
    ThemesRead(Vec<ThemeText>),
    /// A frame was drawn; this is where things ended up.
    Drawn(Geometry),
    /// A store operation failed; the text is safe to show (no message content).
    StoreFailed(String),
    /// Messages read on another device lowered these conversations' unread
    /// counts in the store by this much (D70); answers
    /// `Effect::ReadElsewhere`.
    UnreadCleared(Vec<(ConversationId, u32)>),
    /// Update checks (D148).
    Updates(UpdateEvent),
    /// A deadline from `State::next_deadline` passed.
    Tick,
    /// SIGTERM or SIGHUP (e.g. the tmux pane was closed), or the terminal went away.
    Shutdown,
}

/// What update checks hear back (D148).
#[derive(Debug)]
pub enum UpdateEvent {
    /// What the last run knew (`Effect::LoadUpdateState`): when GitHub was
    /// last asked (seconds since the epoch) and the newest release it named.
    StateLoaded {
        checked_at: Option<i64>,
        latest: Option<Release>,
    },
    /// GitHub's answer to `Effect::CheckForUpdate`; the error is safe to log.
    Checked(Result<Release, String>),
}

/// Work the reducer asks for; run by the effects worker (`effects.rs`, store
/// work) or the command task (`commands.rs`, `Effect::Backend`), whose results
/// come back as `AppEvent`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Answered with `AppEvent::Loaded`.
    LoadConversations,
    /// Answered with `AppEvent::MessagesLoaded`.
    LoadMessages(ConversationId),
    /// Answered with `AppEvent::OlderLoaded`.
    LoadOlder {
        conversation: ConversationId,
        before: MessageRef,
    },
    /// Answered with `AppEvent::MessagesAroundLoaded` (M12).
    LoadMessagesAround {
        conversation: ConversationId,
        target: MessageRef,
    },
    /// Answered with `AppEvent::SearchResults` (M12).
    SearchMessages {
        conversation: Option<ConversationId>,
        query: String,
    },
    /// Answered with `AppEvent::MessageReceiptsLoaded` (M12).
    LoadMessageReceipts {
        conversation: ConversationId,
        message: MessageRef,
    },
    Persist(Box<BackendEvent>),
    /// Record an attachment's status and dimensions in the store (D75).
    SaveAttachment {
        message: MessageRef,
        index: usize,
        attachment: AttachmentRef,
    },
    MarkRead(ConversationId),
    /// Apply messages read on another device to the unread counts (D70);
    /// answered with `AppEvent::UnreadCleared` if any count dropped.
    ReadElsewhere(Vec<MessageRef>),
    /// Store hide/archive/pin/mute/draft for a conversation.
    SaveConversationState(ConversationId, Box<ConversationState>),
    /// Reopen this conversation on the next start.
    RememberOpen(ConversationId),
    /// Store someone's nickname, or remove it (`None`) (D65).
    SaveNickname(ServiceId, Option<String>),
    /// Store the reactions last sent from here, most recent first (D104).
    SaveRecentReactions(Vec<String>),
    /// Read what the last run knew about updates (D148); answered with
    /// `UpdateEvent::StateLoaded`.
    LoadUpdateState,
    /// Remember a successful update check (D148): when (seconds since the
    /// epoch) and the release GitHub named.
    SaveUpdateState {
        checked_at: i64,
        latest: Release,
    },
    /// Ask GitHub for the latest release (D148); answered with
    /// `UpdateEvent::Checked`. Only the terminal app does this.
    CheckForUpdate,
    /// Write a setting to the config file (D48); routed to the config writer.
    SaveSetting(SettingEdit),
    /// Ask the backend to do something; routed to the command task.
    Backend(Command),
    /// Put text on the clipboard (OSC 52; works over SSH and tmux).
    Copy(String),
    /// Open an http(s) link with the system's opener.
    OpenUrl(String),
    /// Open a local file with the system's opener (local session only, D75).
    OpenFile(PathBuf),
    /// Play a downloaded voice note with an external player (local session
    /// only, D81). Answered with `AppEvent::PlaybackFinished`.
    PlayAudio {
        message: MessageRef,
        index: usize,
        path: PathBuf,
        player: String,
    },
    /// Stop any currently playing voice note (D81).
    StopAudio,
    /// Copy a downloaded attachment into `~/Downloads/`, numbering collisions
    /// (D75). Answered with `AppEvent::SavedToDownloads`.
    SaveToDownloads {
        source: PathBuf,
        file_name: Option<String>,
        content_type: String,
    },
    /// List a directory for the palette's file picker (`Level::AttachFile`, D77).
    /// Answered with `AppEvent::FilesListed`.
    ListFiles(String),
    /// Look for a program on `PATH` (the local transcription command, D145).
    /// Answered with `AppEvent::CommandChecked`.
    CheckCommand(String),
    /// Inspect and stage a local file to send in the open conversation (D77).
    /// Answered with `AppEvent::FileStaged`.
    StageFile(PathBuf),
    /// Check off the UI thread whether a single-line pasted path names a
    /// regular file (D77). Answered with `AppEvent::PasteChecked`.
    CheckPaste {
        raw: String,
        path: PathBuf,
    },
    /// Look at the terminal (mosh, kitty); answered with
    /// `AppEvent::TerminalChecked` (D47, D58).
    CheckTerminal,
    /// Read an image from the clipboard for `conversation` (D91, D150). The
    /// app loop pauses input and answers with `AppEvent::ClipboardImage`.
    /// `system`: the computer's own clipboard may be read when the terminal
    /// can't share its own (`[media] paste_from_system`).
    ReadClipboardImage {
        conversation: ConversationId,
        system: bool,
    },
    /// Ring the terminal bell (D49).
    Bell,
    /// Send a desktop notification through the terminal (OSC 99 / OSC 777) or
    /// native OS notifier (DESIGN.md §8).
    DesktopNotify {
        title: String,
        body: String,
    },
    /// `Escape` had nothing left to dismiss (WEBSITE.md §4.3): the host may
    /// release keyboard focus. The terminal app ignores it (faro keeps its
    /// own focus); the web demo blurs the canvas so the page takes keys.
    ReleaseFocus,
}

/// Work for the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Answered with `AppEvent::SendResult`. The message is already stored
    /// locally as `Sending`.
    Send {
        conversation: ConversationId,
        message: OutgoingMessage,
    },
    /// Answered with `AppEvent::ReactResult`. Already applied locally;
    /// `previous` is what to restore if it fails.
    React {
        conversation: ConversationId,
        target: MessageRef,
        change: ReactionChange,
        previous: Option<String>,
    },
    /// Answered with `AppEvent::EditResult`. Already applied locally.
    Edit {
        conversation: ConversationId,
        target: MessageRef,
        edit: OutgoingEdit,
    },
    /// Answered with `AppEvent::DeleteResult`. Already applied locally.
    DeleteForEveryone {
        conversation: ConversationId,
        target: MessageRef,
    },
    /// Download an attachment and write it to `attachments/` (`0600`, D75).
    /// Answered with `AppEvent::Fetched`.
    Fetch {
        conversation: ConversationId,
        message: MessageRef,
        index: usize,
        attachment: AttachmentRef,
    },
    /// Transcribe a downloaded voice note (`local` or `remote`, D82).
    /// Answered with `AppEvent::Transcribed`.
    Transcribe {
        conversation: ConversationId,
        message: MessageRef,
        index: usize,
        path: PathBuf,
        /// How long the note is, if known: the local timeout grows with it (D145).
        duration_secs: Option<u32>,
        voice: Voice,
    },
    /// Read receipts for messages that have been seen (D42). No answer;
    /// failures are only logged.
    MarkRead(Vec<MessageRef>),
    /// Our typing indicator (D42). No answer; failures are only logged.
    Typing {
        conversation: ConversationId,
        state: TypingState,
    },
}

/// Why no image was staged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardError {
    /// faro runs over SSH and the terminal can't share its clipboard (D150).
    OverSsh,
    /// The terminal can't share its clipboard and reading the computer's own
    /// is turned off (`[media] paste_from_system = false`, D150).
    SystemOff,
    /// No `wl-paste` or `xclip` to read the computer's clipboard with.
    NeedsTool,
    /// No desktop session (no Wayland or X11 display) to read a clipboard
    /// from.
    NoDesktop,
    /// mosh relays the screen, not the bytes: the request never arrives.
    Mosh,
    /// The clipboard has no image type faro can send.
    NoImage,
    /// The type list never came.
    NoAnswer,
    /// The prompt wasn't answered in time.
    PromptTimedOut,
    Read(ReadError),
    /// Reading the tty or saving the file failed.
    Failed(String),
}

impl ClipboardError {
    /// What the footer says.
    pub fn notice(&self) -> Notice {
        match self {
            Self::OverSsh => {
                Notice::warning("Pasting images over SSH needs kitty or a newer Ghostty")
            }
            Self::SystemOff => Notice::warning(
                "This terminal can't paste images (Settings › Paste from the system clipboard is off)",
            ),
            Self::NeedsTool => Notice::warning("Install wl-clipboard (or xclip) to paste images"),
            Self::NoDesktop => Notice::warning("No desktop clipboard to paste images from"),
            Self::Mosh => Notice::warning("Pasting images doesn't work over mosh"),
            Self::NoImage => Notice::info("No image on the clipboard"),
            Self::NoAnswer => Notice::warning("No answer from the terminal"),
            Self::PromptTimedOut => {
                Notice::warning("No answer: the clipboard prompt wasn't answered")
            }
            Self::Read(ReadError::Denied) => Notice::info("Clipboard access denied"),
            Self::Read(ReadError::Busy) => Notice::warning("The clipboard is busy; try again"),
            Self::Read(ReadError::Unavailable) => {
                Notice::warning("The terminal can't share its clipboard here")
            }
            Self::Read(ReadError::TooLarge { limit }) => Notice::warning(format!(
                "Image too large (over {})",
                faro_core::domain::format_byte_size(u64::try_from(*limit).unwrap_or(u64::MAX))
            )),
            Self::Read(ReadError::Malformed(_) | ReadError::Incomplete) => {
                Notice::warning("The image didn't arrive intact; try again")
            }
            Self::Read(error) => Notice::warning(format!("Couldn't paste the image: {error}")),
            Self::Failed(error) => Notice::error(format!("Couldn't paste the image: {error}")),
        }
    }
}
