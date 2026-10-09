//! What flows into the reducer (`AppEvent`) and out of it (`Effect`), D22.

use std::path::PathBuf;

use faro_core::backend::{BackendError, BackendEvent};
use faro_core::config::SettingEdit;
use faro_core::domain::{
    AttachmentRef, ConversationId, Message, MessageRef, ServiceId, Timestamp, TranscriptSource,
};
use faro_core::store::{ConversationState, ConversationSummary, RecipientReceipt, SearchHit};

use super::commands::Command;
use super::state::Geometry;
use crate::theme::ThemeText;

#[derive(Debug)]
pub enum AppEvent {
    Terminal(crossterm::event::Event),
    Backend(BackendEvent),
    /// Startup data from the store.
    Loaded {
        conversations: Vec<ConversationSummary>,
        last_open: Option<ConversationId>,
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
        result: Result<super::state::StagedAttachment, super::clipboard::ClipboardError>,
    },
    /// A late clipboard reply was drained (D90); input resumes.
    ClipboardDrained,
    /// An inline image finished decoding and slicing off the UI thread (D79).
    ImageDecoded(super::images::DecodedImage),
    /// External audio playback finished or failed (D81).
    PlaybackFinished {
        message: MessageRef,
        index: usize,
        error: Option<String>,
    },
    /// Opening a link or attachment failed; the text is safe to show.
    OpenFailed(String),
    /// What's between faro and the user: mosh (D47), and who draws block
    /// glyphs (D58).
    TerminalChecked(super::system::Terminal),
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
    /// A deadline from `State::next_deadline` passed.
    Tick,
    /// SIGTERM or SIGHUP (e.g. the tmux pane was closed), or the terminal went away.
    Shutdown,
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
    /// Read an image from the clipboard for `conversation` (D91). The app
    /// loop pauses input and answers with `AppEvent::ClipboardImage`.
    ReadClipboardImage {
        conversation: ConversationId,
    },
    /// Ring the terminal bell (D49).
    Bell,
    /// Send a desktop notification through the terminal (OSC 99 / OSC 777) or
    /// native OS notifier (DESIGN.md §8).
    DesktopNotify {
        title: String,
        body: String,
    },
}
