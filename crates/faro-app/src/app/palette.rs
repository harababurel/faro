//! The command palette (D44, tree since D52): a tree of levels. The root
//! lists the categories (chats, themes, settings, commands, keys); typing
//! filters only the level shown, `Enter`/`→` opens a category, `←` (or
//! `Backspace` on an empty query) goes back up. At the root, `@` `:` `>` `?`
//! jump straight into chats, settings, commands and keys. Settings is one
//! list under section headers that can't be selected (D57). A level's items are
//! built from the state when it's shown; matching is synchronous
//! (`nucleo-matcher`): a few hundred short strings take well under a
//! millisecond.

use std::path::PathBuf;

use faro_core::config::{
    AutoDownload, Avatars, BubbleStyle, Choice as _, ColorChoice, DesktopNotifications,
    EmojiChoice, InlineImages, LogoBeam, MessageDensity, NotificationPrivacy, SIDEBAR_WIDTH_MAX,
    SIDEBAR_WIDTH_MIN, TimestampFormat, TranscribeMode, key,
};
use faro_core::domain::{ConversationId, MessageRef, ServiceId};
use faro_core::store::{NotifyLevel, RemoteTranscription, SearchHit};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::input::Input;
use super::keymap::{Action, Context, Curate, Keymap};
use super::state::{
    ConversationAction, FileEntry, Focus, GraphicsBlocker, MUTE_CHOICES, MessageLayout, Section,
    SidebarView, State,
};
use super::updates::Unavailable;
use crate::theme::{ColorDepth, Theme};

/// How many recently used commands are remembered (and ranked higher).
pub const RECENT_MAX: usize = 10;
/// Score bonus per place in the recent list (a strong match still wins).
const RECENT_BONUS: u32 = 3;
/// Recently used commands shown first when nothing is typed.
const RECENT_SHOWN: usize = 3;
/// "Hide conversations inactive for…" choices: days and label.
pub const INACTIVE_CHOICES: [(u32, &str); 4] = [
    (7, "1 week"),
    (30, "1 month"),
    (90, "3 months"),
    (365, "1 year"),
];

/// Local Whisper models (D145): name, and what to expect. All multilingual.
pub const LOCAL_MODELS: &[(&str, &str)] = &[
    ("tiny", "fastest, roughest · 75 MB"),
    ("base", "fast · 140 MB"),
    ("small", "balanced · 460 MB"),
    ("medium", "accurate, slow · 1.5 GB"),
    ("turbo", "most accurate · 1.6 GB, best with a GPU"),
];

/// Local transcription commands (D145).
pub const TRANSCRIBE_COMMANDS: &[(&str, &str)] = &[
    ("whisper", "OpenAI Whisper"),
    ("whisper-cli", "whisper.cpp: the model is a file"),
];

const OPENAI_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const GROQ_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";

/// OpenAI-compatible transcription endpoints (D145): URL, name, host.
pub const TRANSCRIBE_ENDPOINTS: &[(&str, &str, &str)] = &[
    (OPENAI_URL, "OpenAI", "api.openai.com"),
    (GROQ_URL, "Groq", "api.groq.com"),
];

/// The models an endpoint offers, best first (D145); none for one faro
/// doesn't know (`Custom…` types it).
pub fn remote_models(url: &str) -> &'static [(&'static str, &'static str)] {
    match url {
        OPENAI_URL => &[
            ("gpt-4o-mini-transcribe", "accurate, cheapest"),
            ("gpt-4o-transcribe", "most accurate"),
            ("whisper-1", "the original Whisper"),
        ],
        GROQ_URL => &[
            ("whisper-large-v3-turbo", "fast, cheap"),
            ("whisper-large-v3", "most accurate"),
        ],
        _ => &[],
    }
}

/// Languages to hint at (D145): ISO 639-1 code (empty: detect), name.
pub const TRANSCRIBE_LANGUAGES: &[(&str, &str)] = &[
    ("", "auto-detect"),
    ("ar", "Arabic"),
    ("zh", "Chinese"),
    ("nl", "Dutch"),
    ("en", "English"),
    ("fr", "French"),
    ("de", "German"),
    ("hi", "Hindi"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("es", "Spanish"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
];

/// The longest voice note transcribed automatically (D145), in seconds.
/// The sidebar widths offered (D156); `Custom…` takes any other.
pub const SIDEBAR_WIDTHS: &[(u16, &str)] = &[
    (24, "24 (narrow)"),
    (30, "30 (default)"),
    (40, "40 (wide)"),
    (50, "50 (wider)"),
];

pub const TRANSCRIBE_MAX_SECS: &[(u32, &str)] = &[
    (30, "30 s"),
    (60, "1 min"),
    (120, "2 min"),
    (300, "5 min"),
    (600, "10 min"),
    (1800, "30 min"),
];

/// Idle auto-lock timeout choices (`[security] auto_lock_mins`, D168).
pub const AUTO_LOCK_MINS: &[(u32, &str)] = &[
    (0, "off"),
    (1, "1 min"),
    (5, "5 min"),
    (15, "15 min"),
    (30, "30 min"),
    (60, "1 hour"),
];

/// A level of the palette's tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The categories.
    Root,
    Chats,
    /// Full-text search in `Some(id)` or across all conversations (`None`).
    Search(Option<ConversationId>),
    Themes,
    Settings,
    /// One setting's values (`Settings › Colours`).
    Setting(Setting),
    Commands,
    /// The keybinding list.
    Keys,
    /// Everyone faro knows, to give one a nickname (D65).
    People,
    /// Typing a nickname for someone: the query is the name (D65).
    Nickname(ServiceId),
    /// Typing a setting's value (`Custom…`): the query is the value (D145).
    SettingText(Setting),
    /// Browsing a directory to attach a file to the open conversation (D77).
    AttachFile,
    /// Shared links and files in the open conversation (M12).
    ChatLinks,
    /// Pick a conversation to forward a message to.
    ForwardTo(MessageRef),
    /// In-app release notes and the "what's new since you" digest.
    ReleaseNotes,
    /// Linked devices on this account, with last-seen and unlink.
    Devices,
    /// Diagnostic report: terminal capabilities, graphics fallback, config & keymap (`#47`).
    Doctor,
}

/// The sections of `Settings ›`, each under a header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsGroup {
    Appearance,
    Messages,
    /// Settings that only change the bubbles layout (D99).
    Bubbles,
    Notifications,
    Conversations,
    Media,
    /// Transcription: what applies depends on the mode (D145).
    VoiceNotes,
    Input,
    /// Idle auto-lock (`[security]`, D168).
    Security,
    /// Update checks (D148).
    Updates,
}

/// A setting: on/off ones flip where they are, the others list their
/// values as a level of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Colors,
    Emoji,
    /// `[ui] sidebar_width` (D156).
    SidebarWidth,
    Layout,
    BubbleStyle,
    BubblePadding,
    MatchWidths,
    Density,
    Timestamps,
    ReduceMotion,
    Beam,
    Bell,
    Title,
    Desktop,
    Privacy,
    Allowlist,
    /// `[conversations] restore_session`.
    RestoreSession,
    /// `[conversations] previews` and the two that only apply with it (D154).
    Previews,
    PreviewTime,
    PreviewActivity,
    /// `[conversations] avatars` (D158).
    Avatars,
    InlineImages,
    AutoDownload,
    Transcribe,
    TranscribeAuto,
    TranscribeMaxSecs,
    TranscribeModel,
    TranscribeLanguage,
    TranscribeCommand,
    TranscribeEndpoint,
    TranscribeRemoteModel,
    Mouse,
    /// `[media] paste_from_system` (D150).
    PasteFromSystem,
    /// `[security] auto_lock_mins` (D168).
    AutoLock,
    UpdateCheck,
}

impl Level {
    /// The root's categories, in order (with `Search` scoped to `open`).
    pub const fn categories(open: Option<ConversationId>) -> [Self; 6] {
        [
            Self::Chats,
            Self::Themes,
            Self::Settings,
            Self::Commands,
            Self::Keys,
            Self::Search(open),
        ]
    }

    /// What the breadcrumb (and the item that opens it) calls it.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Root => "Command palette",
            Self::Chats => "Chats",
            Self::Search(_) => "Search",
            Self::Themes => "Themes",
            Self::Settings => "Settings",
            Self::Setting(setting) => setting.title(),
            Self::Commands => "Commands",
            Self::Keys => "Keys",
            Self::People => "Nicknames",
            Self::Nickname(_) => "Nickname",
            Self::SettingText(_) => "Custom",
            Self::AttachFile => "Send a file",
            Self::ChatLinks => "Links & files",
            Self::ForwardTo(_) => "Forward to",
            Self::ReleaseNotes => "Release notes",
            Self::Devices => "Linked devices",
            Self::Doctor => "Doctor",
        }
    }

    /// The character that jumps here from the root.
    pub const fn prefix(self) -> Option<char> {
        match self {
            Self::Chats => Some('@'),
            Self::Search(_) => Some('/'),
            Self::Settings => Some(':'),
            Self::Commands => Some('>'),
            Self::Keys => Some('?'),
            Self::Root
            | Self::Themes
            | Self::Setting(_)
            | Self::People
            | Self::Nickname(_)
            | Self::SettingText(_)
            | Self::AttachFile
            | Self::ChatLinks
            | Self::ForwardTo(_)
            | Self::ReleaseNotes
            | Self::Devices
            | Self::Doctor => None,
        }
    }

    /// The category a character typed first at the root jumps to.
    pub fn for_prefix(c: char, open: Option<ConversationId>) -> Option<Self> {
        Self::categories(open)
            .into_iter()
            .find(|level| level.prefix() == Some(c))
    }

    /// String form saved in the session record (`ui.session`).
    pub fn to_stored(self) -> String {
        match self {
            Self::Root => "root".to_owned(),
            Self::Chats => "chats".to_owned(),
            Self::Search(None) => "search".to_owned(),
            Self::Search(Some(id)) => format!("search {id}"),
            Self::Themes => "themes".to_owned(),
            Self::Settings => "settings".to_owned(),
            Self::Setting(s) => format!("setting {}", s.key()),
            Self::Commands => "commands".to_owned(),
            Self::Keys => "keys".to_owned(),
            Self::People => "people".to_owned(),
            Self::Nickname(id) => format!("nickname {id}"),
            Self::SettingText(s) => format!("setting_text {}", s.key()),
            Self::AttachFile => "attach_file".to_owned(),
            Self::ChatLinks => "chat_links".to_owned(),
            Self::ForwardTo(m) => format!("forward_to {} {}", m.sender, m.sent_at.as_millis()),
            Self::ReleaseNotes => "release_notes".to_owned(),
            Self::Devices => "devices".to_owned(),
            Self::Doctor => "doctor".to_owned(),
        }
    }

    /// Parse the string form written by [`Self::to_stored`].
    pub fn from_stored(text: &str) -> Option<Self> {
        let mut parts = text.split_whitespace();
        match parts.next()? {
            "root" => Some(Self::Root),
            "chats" => Some(Self::Chats),
            "search" => match parts.next() {
                None => Some(Self::Search(None)),
                Some(id) => Some(Self::Search(Some(id.parse().ok()?))),
            },
            "themes" => Some(Self::Themes),
            "settings" => Some(Self::Settings),
            "setting" => Some(Self::Setting(Setting::from_key(parts.next()?)?)),
            "commands" => Some(Self::Commands),
            "keys" => Some(Self::Keys),
            "people" => Some(Self::People),
            "nickname" => Some(Self::Nickname(parts.next()?.parse().ok()?)),
            "setting_text" => Some(Self::SettingText(Setting::from_key(parts.next()?)?)),
            "attach_file" => Some(Self::AttachFile),
            "chat_links" => Some(Self::ChatLinks),
            "forward_to" => {
                let sender = parts.next()?.parse().ok()?;
                let sent_at =
                    faro_core::domain::Timestamp::from_millis(parts.next()?.parse().ok()?);
                Some(Self::ForwardTo(MessageRef { sender, sent_at }))
            }
            "release_notes" => Some(Self::ReleaseNotes),
            "devices" => Some(Self::Devices),
            "doctor" => Some(Self::Doctor),
            _ => None,
        }
    }
}

impl SettingsGroup {
    pub const ALL: [Self; 10] = [
        Self::Appearance,
        Self::Messages,
        Self::Bubbles,
        Self::Notifications,
        Self::Conversations,
        Self::Media,
        Self::VoiceNotes,
        Self::Input,
        Self::Security,
        Self::Updates,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Messages => "Messages",
            Self::Bubbles => "Bubbles",
            Self::Notifications => "Notifications",
            Self::Conversations => "Conversations",
            Self::Media => "Media",
            Self::VoiceNotes => "Voice notes",
            Self::Input => "Input",
            Self::Security => "Security",
            Self::Updates => "Updates",
        }
    }

    /// Its settings, in order. Themes are a category of their own at the
    /// root, not a row here (D60).
    pub(crate) const fn settings(self) -> &'static [Setting] {
        match self {
            Self::Appearance => &[
                Setting::Colors,
                Setting::Emoji,
                Setting::SidebarWidth,
                Setting::ReduceMotion,
                Setting::Beam,
            ],
            Self::Messages => &[Setting::Layout, Setting::Timestamps],
            Self::Bubbles => &[
                Setting::BubbleStyle,
                Setting::BubblePadding,
                Setting::MatchWidths,
                Setting::Density,
            ],
            Self::Notifications => &[
                Setting::Bell,
                Setting::Title,
                Setting::Desktop,
                Setting::Privacy,
            ],
            Self::Conversations => &[
                Setting::Allowlist,
                Setting::RestoreSession,
                Setting::Previews,
                Setting::PreviewTime,
                Setting::PreviewActivity,
                Setting::Avatars,
            ],
            Self::Media => &[
                Setting::InlineImages,
                Setting::AutoDownload,
                Setting::PasteFromSystem,
            ],
            Self::VoiceNotes => &[
                Setting::Transcribe,
                Setting::TranscribeAuto,
                Setting::TranscribeMaxSecs,
                Setting::TranscribeModel,
                Setting::TranscribeCommand,
                Setting::TranscribeEndpoint,
                Setting::TranscribeRemoteModel,
                Setting::TranscribeLanguage,
            ],
            Self::Input => &[Setting::Mouse],
            Self::Security => &[Setting::AutoLock],
            Self::Updates => &[Setting::UpdateCheck],
        }
    }
}

impl Setting {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Colors => "Colours",
            Self::Emoji => "Emoji",
            Self::SidebarWidth => "Sidebar width",
            Self::Layout => "Message layout",
            Self::BubbleStyle => "Bubble style",
            Self::BubblePadding => "Bubble padding",
            Self::MatchWidths => "Match bubble widths",
            Self::Density => "Message density",
            Self::Timestamps => "Timestamps",
            Self::ReduceMotion => "Reduce motion",
            Self::Beam => "Logo beam",
            Self::Bell => "Bell",
            Self::Title => "Unread count in the window title",
            Self::Desktop => "Desktop notifications",
            Self::Privacy => "Notification privacy",
            Self::Allowlist => "Allowlist mode",
            Self::RestoreSession => "Restore last session",
            Self::Previews => "Message previews",
            Self::PreviewTime => "Time in the list",
            Self::PreviewActivity => "Typing and drafts",
            Self::Avatars => "Avatars",
            Self::InlineImages => "Inline images",
            Self::AutoDownload => "Auto-download media",
            Self::Transcribe => "Transcription",
            Self::TranscribeAuto => "Auto-transcribe",
            Self::TranscribeMaxSecs => "Auto-transcribe notes up to",
            Self::TranscribeModel => "Model",
            Self::TranscribeLanguage => "Language",
            Self::TranscribeCommand => "Command",
            Self::TranscribeEndpoint => "Endpoint",
            Self::TranscribeRemoteModel => "Remote model",
            Self::Mouse => "Mouse",
            Self::PasteFromSystem => "Paste from the system clipboard",
            Self::AutoLock => "Auto-lock after inactivity",
            Self::UpdateCheck => "Check for updates",
        }
    }

    /// Its key in the config file.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Colors => key::COLORS,
            Self::Emoji => key::EMOJI,
            Self::SidebarWidth => key::SIDEBAR_WIDTH,
            Self::Layout => key::LAYOUT,
            Self::BubbleStyle => key::BUBBLE_STYLE,
            Self::BubblePadding => key::BUBBLE_PADDING,
            Self::MatchWidths => key::MATCH_WIDTHS,
            Self::Density => key::DENSITY,
            Self::Timestamps => key::TIMESTAMPS,
            Self::ReduceMotion => key::REDUCE_MOTION,
            Self::Beam => key::BEAM,
            Self::Bell => key::BELL,
            Self::Title => key::TITLE,
            Self::Desktop => key::DESKTOP,
            Self::Privacy => key::PRIVACY,
            Self::Allowlist => key::ALLOWLIST,
            Self::RestoreSession => key::RESTORE_SESSION,
            Self::Previews => key::PREVIEWS,
            Self::PreviewTime => key::PREVIEW_TIME,
            Self::PreviewActivity => key::PREVIEW_ACTIVITY,
            Self::Avatars => key::AVATARS,
            Self::InlineImages => key::INLINE_IMAGES,
            Self::AutoDownload => key::AUTO_DOWNLOAD,
            Self::Transcribe => key::VOICE_TRANSCRIBE,
            Self::TranscribeAuto => key::VOICE_AUTO_TRANSCRIBE,
            Self::TranscribeMaxSecs => key::VOICE_TRANSCRIBE_MAX_SECS,
            Self::TranscribeModel => key::VOICE_TRANSCRIBE_MODEL,
            Self::TranscribeLanguage => key::VOICE_TRANSCRIBE_LANGUAGE,
            Self::TranscribeCommand => key::VOICE_TRANSCRIBE_COMMAND,
            Self::TranscribeEndpoint => key::VOICE_TRANSCRIBE_URL,
            Self::TranscribeRemoteModel => key::VOICE_TRANSCRIBE_REMOTE_MODEL,
            Self::Mouse => key::MOUSE,
            Self::PasteFromSystem => key::PASTE_FROM_SYSTEM,
            Self::AutoLock => key::AUTO_LOCK_MINS,
            Self::UpdateCheck => key::UPDATES_CHECK,
        }
    }

    /// Look up a setting by its config key.
    pub fn from_key(key: &str) -> Option<Self> {
        SettingsGroup::ALL
            .iter()
            .flat_map(|g| g.settings().iter().copied())
            .find(|s| s.key() == key)
    }

    /// Whether it differs from its default (marked in the settings list,
    /// D100).
    pub fn changed(self, state: &State) -> bool {
        state.config.value(self.key()) != faro_core::config::Config::default().value(self.key())
    }

    /// Why it does nothing with the other settings as they are, if it
    /// doesn't: dimmed and skipped in the settings list with this reason on
    /// the right. Bubble settings under the IRC layout (D99); transcription
    /// settings for the other mode, or with transcription off (D145).
    pub fn unavailable(self, state: &State) -> Option<&'static str> {
        let mode = state.config.voice.transcribe;
        match self {
            Self::BubbleStyle | Self::BubblePadding | Self::MatchWidths | Self::Density => {
                (state.message_layout() != MessageLayout::Bubbles).then_some("bubbles layout only")
            }
            Self::TranscribeAuto | Self::TranscribeLanguage => {
                (mode == TranscribeMode::Off).then_some("transcription is off")
            }
            Self::TranscribeMaxSecs => {
                if mode == TranscribeMode::Off {
                    Some("transcription is off")
                } else if !state.config.voice.auto_transcribe {
                    Some("when transcribing automatically")
                } else {
                    None
                }
            }
            Self::TranscribeModel | Self::TranscribeCommand => match mode {
                TranscribeMode::Off => Some("transcription is off"),
                TranscribeMode::Remote => Some("local only"),
                TranscribeMode::Local => None,
            },
            Self::TranscribeEndpoint | Self::TranscribeRemoteModel => match mode {
                TranscribeMode::Off => Some("transcription is off"),
                TranscribeMode::Local => Some("remote only"),
                TranscribeMode::Remote => None,
            },
            Self::UpdateCheck => state.update_checks_unavailable().map(Unavailable::reason),
            Self::PreviewTime | Self::PreviewActivity => {
                (!state.config.conversations.previews).then_some("needs message previews")
            }
            Self::Colors
            | Self::Emoji
            | Self::Layout
            | Self::Timestamps
            | Self::ReduceMotion
            | Self::Beam
            | Self::Bell
            | Self::Title
            | Self::Desktop
            | Self::Privacy
            | Self::Allowlist
            | Self::RestoreSession
            | Self::Avatars
            | Self::SidebarWidth
            | Self::Previews
            | Self::InlineImages
            | Self::AutoDownload
            | Self::Transcribe
            | Self::Mouse
            | Self::PasteFromSystem
            | Self::AutoLock => None,
        }
    }

    /// Whether its value can be typed (`Custom…`) besides chosen from a list.
    pub const fn takes_text(self) -> bool {
        match self {
            Self::TranscribeModel
            | Self::TranscribeLanguage
            | Self::TranscribeCommand
            | Self::TranscribeEndpoint
            | Self::TranscribeRemoteModel
            | Self::SidebarWidth => true,
            Self::Colors
            | Self::Emoji
            | Self::Layout
            | Self::BubbleStyle
            | Self::BubblePadding
            | Self::MatchWidths
            | Self::Density
            | Self::Timestamps
            | Self::ReduceMotion
            | Self::Beam
            | Self::Bell
            | Self::Title
            | Self::Desktop
            | Self::Privacy
            | Self::Allowlist
            | Self::RestoreSession
            | Self::Avatars
            | Self::Previews
            | Self::PreviewTime
            | Self::PreviewActivity
            | Self::InlineImages
            | Self::AutoDownload
            | Self::Transcribe
            | Self::TranscribeAuto
            | Self::TranscribeMaxSecs
            | Self::Mouse
            | Self::PasteFromSystem
            | Self::AutoLock
            | Self::UpdateCheck => false,
        }
    }

    /// Whether it's on or off (flipped by `Enter` in the settings list).
    pub const fn is_switch(self) -> bool {
        match self {
            Self::BubblePadding
            | Self::MatchWidths
            | Self::ReduceMotion
            | Self::Bell
            | Self::Title
            | Self::Allowlist
            | Self::RestoreSession
            | Self::Previews
            | Self::PreviewTime
            | Self::PreviewActivity
            | Self::TranscribeAuto
            | Self::Mouse
            | Self::PasteFromSystem
            | Self::UpdateCheck => true,
            Self::Colors
            | Self::Emoji
            | Self::SidebarWidth
            | Self::Beam
            | Self::Avatars
            | Self::Layout
            | Self::BubbleStyle
            | Self::Density
            | Self::Timestamps
            | Self::Desktop
            | Self::Privacy
            | Self::InlineImages
            | Self::AutoDownload
            | Self::Transcribe
            | Self::TranscribeMaxSecs
            | Self::TranscribeModel
            | Self::TranscribeLanguage
            | Self::TranscribeCommand
            | Self::TranscribeEndpoint
            | Self::TranscribeRemoteModel
            | Self::AutoLock => false,
        }
    }

    /// Whether moving through its values draws each one before it's chosen
    /// (D59): the ones that only change how things are drawn. Emoji isn't
    /// one: switching modes repaints everything and `auto` waits for a look
    /// at the terminal.
    pub const fn previews(self) -> bool {
        match self {
            Self::Colors
            | Self::SidebarWidth
            | Self::Beam
            | Self::Avatars
            | Self::Layout
            | Self::BubbleStyle
            | Self::Density
            | Self::Timestamps => true,
            Self::Emoji
            | Self::BubblePadding
            | Self::MatchWidths
            | Self::ReduceMotion
            | Self::Bell
            | Self::Title
            | Self::Desktop
            | Self::Privacy
            | Self::Allowlist
            | Self::RestoreSession
            | Self::Previews
            | Self::PreviewTime
            | Self::PreviewActivity
            | Self::InlineImages
            | Self::AutoDownload
            | Self::Transcribe
            | Self::TranscribeAuto
            | Self::TranscribeMaxSecs
            | Self::TranscribeModel
            | Self::TranscribeLanguage
            | Self::TranscribeCommand
            | Self::TranscribeEndpoint
            | Self::TranscribeRemoteModel
            | Self::Mouse
            | Self::PasteFromSystem
            | Self::AutoLock
            | Self::UpdateCheck => false,
        }
    }
}

/// What the palette tries on while it's open (D54, D59).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    Theme(String),
    Setting(SettingChoice),
}

/// What a command item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteCommand {
    /// The same as a key.
    Action(Action),
    /// To the open conversation.
    Conversation(ConversationAction),
    /// Mute the open conversation for `MUTE_CHOICES[i]`.
    MuteFor(usize),
    /// Hide every conversation inactive for `INACTIVE_CHOICES[i]`.
    HideInactive(usize),
    /// Copy the local file path of a downloaded attachment on the selected message.
    CopyAttachmentPath,
    /// Open the conversation's media gallery (`Popup::ImageViewer`, #21).
    MediaGallery,
    /// Open message and transcript search (`Level::Search`).
    SearchMessages,
    /// Jump to the unread marker in the open conversation.
    JumpUnread,
    /// Show the keybinding list.
    ShowKeys,
    /// Show `Popup::About` (D94).
    About,
    /// Open in-app release notes in the palette (`Level::ReleaseNotes`), or
    /// fall back to opening/copying the GitHub tag URL (D148).
    ReleaseNotes,
    /// Open (or copy, over SSH) the GitHub release tag URL in a browser.
    OpenReleaseUrl,
}

/// A value for a setting (D48); choosing it applies it and saves it to the
/// config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingChoice {
    Colors(ColorChoice),
    /// Columns (D156); typed values go through `ItemKind::SetText`.
    SidebarWidth(u16),
    Layout(MessageLayout),
    BubbleStyle(BubbleStyle),
    BubblePadding(bool),
    MatchWidths(bool),
    Density(MessageDensity),
    Timestamps(TimestampFormat),
    Allowlist(bool),
    RestoreSession(bool),
    Previews(bool),
    PreviewTime(bool),
    PreviewActivity(bool),
    Avatars(Avatars),
    Emoji(EmojiChoice),
    Desktop(DesktopNotifications),
    Privacy(NotificationPrivacy),
    InlineImages(InlineImages),
    AutoDownload(AutoDownload),
    Transcribe(TranscribeMode),
    TranscribeAuto(bool),
    TranscribeMaxSecs(u32),
    /// A preset; typed values go through `ItemKind::SetText` (D145).
    TranscribeModel(&'static str),
    TranscribeLanguage(&'static str),
    TranscribeCommand(&'static str),
    TranscribeEndpoint(&'static str),
    TranscribeRemoteModel(&'static str),
    Mouse(bool),
    PasteFromSystem(bool),
    Bell(bool),
    Title(bool),
    ReduceMotion(bool),
    AutoLock(u32),
    Beam(LogoBeam),
    UpdateCheck(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    /// A category or sub-level: choosing it opens it.
    Level(Level),
    Command(PaletteCommand),
    Conversation(ConversationId),
    Setting(SettingChoice),
    /// An on/off setting in the settings list: choosing it flips it and the
    /// palette stays open.
    Switch(Setting),
    /// A section's title; never selected, kept while a match is under it.
    Header,
    /// A theme by name (the config's `theme`).
    Theme(String),
    /// A line of the keybinding list (for reference; choosing it closes).
    Key,
    /// Give this person the typed name as a nickname (an empty query
    /// removes it) (D65).
    SetNickname(ServiceId),
    /// Take this person's nickname away.
    ClearNickname(ServiceId),
    /// Save what's typed as this setting's value (`Level::SettingText`, D145).
    SetText(Setting),
    /// Enter a subdirectory in `Level::AttachFile` by setting the query to this path.
    EnterDir(String),
    /// Stage a file for sending from `Level::AttachFile` (D77).
    AttachFile(PathBuf),
    /// A message or transcript search result (`Level::Search`, M12).
    SearchHit {
        conversation: ConversationId,
        message: MessageRef,
    },
    /// A saved/pinned search query in `Level::Search` (#26).
    SavedQuery(String),
    /// A shared URL in the open conversation (`Level::ChatLinks`, M12).
    ChatLink {
        message: MessageRef,
        url: String,
    },
    /// An attachment in the open conversation (`Level::ChatLinks`, M12).
    ChatAttachment {
        message: MessageRef,
        index: usize,
    },
    /// Forward `target` to `destination` (`Level::ForwardTo`, M12).
    ForwardTo {
        target: MessageRef,
        destination: ConversationId,
    },
    /// A linked device (`Level::Devices`).
    Device(u32),
    /// A row in `Level::Doctor`: choosing it copies the full diagnostic report.
    DoctorRow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: ItemKind,
    /// What's matched and shown.
    pub title: String,
    /// Shown dim before the title (`Alice › `, `Everywhere › `); not matched.
    pub path: Option<String>,
    /// Right-aligned: its key, a note (`3 unread · archived`) or a value.
    pub detail: String,
    /// Place in the recently used commands.
    pub recent: Option<usize>,
    /// For one of a setting's values: whether it's the one in use (drawn as
    /// a radio button).
    pub current: Option<bool>,
    /// Shown dimmed and never selected: a setting that does nothing right
    /// now (D99).
    pub disabled: bool,
    /// A setting that differs from its default: marked before its value
    /// (D100).
    pub changed: bool,
}

impl Item {
    fn new(kind: ItemKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            path: None,
            detail: String::new(),
            recent: None,
            current: None,
            disabled: false,
            changed: false,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    fn current(mut self, current: bool) -> Self {
        self.current = Some(current);
        self
    }

    /// Whether it can be selected (headers and disabled items can't).
    pub fn selectable(&self) -> bool {
        self.kind != ItemKind::Header && !self.disabled
    }
}

/// An item that matches, with the title's matched characters (grapheme
/// indices, for highlighting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub item: usize,
    pub highlight: Vec<u32>,
}

#[derive(Debug)]
pub struct Palette {
    /// From the root down to the level shown (never empty).
    pub path: Vec<Level>,
    pub input: Input,
    /// The level's items.
    pub items: Vec<Item>,
    /// What matches the query, best first.
    pub matches: Vec<Match>,
    /// Index into `matches`.
    pub selected: usize,
    /// First result shown (the renderer keeps `selected` in view).
    pub offset: usize,
    /// What's drawn while the palette is open: the theme selected at the
    /// Themes level (D54) or the value selected at a setting that changes
    /// how things look (D59), tried on before it's chosen.
    pub preview: Option<Preview>,
    /// A line above the results about the level (the colour depth at
    /// Themes, D57).
    pub note: Option<String>,
    /// Latest SQLite search results for `Level::Search` (M12).
    pub search_hits: Vec<SearchHit>,
    matcher: Matcher,
}

impl Palette {
    pub fn new(path: Vec<Level>, items: Vec<Item>) -> Self {
        let mut palette = Self {
            path: Vec::new(),
            input: Input::new(""),
            items: Vec::new(),
            matches: Vec::new(),
            selected: 0,
            offset: 0,
            preview: None,
            note: None,
            search_hits: Vec::new(),
            matcher: Matcher::new(Config::DEFAULT),
        };
        palette.show(path, items, "", None);
        palette
    }

    /// What's typed in the palette's input box.
    pub fn query(&self) -> &str {
        self.input.text()
    }

    /// The level shown.
    pub fn level(&self) -> Level {
        self.path.last().copied().unwrap_or(Level::Root)
    }

    /// The palette's title: where in the tree it is (`Settings › Bell`).
    pub fn breadcrumb(&self) -> String {
        let below_root: Vec<&str> = self
            .path
            .iter()
            .filter(|level| **level != Level::Root)
            .map(|level| level.title())
            .collect();
        if below_root.is_empty() {
            Level::Root.title().to_owned()
        } else {
            below_root.join(" › ")
        }
    }

    /// Show another level with `items`, the query set to `query`, and the
    /// item of kind `select` selected if it matches (else the best match).
    pub fn show(
        &mut self,
        path: Vec<Level>,
        items: Vec<Item>,
        query: &str,
        select: Option<&ItemKind>,
    ) {
        self.path = if path.is_empty() {
            vec![Level::Root]
        } else {
            path
        };
        self.items = items;
        self.input = Input::new(query);
        self.refilter();
        if let Some(kind) = select {
            self.select(kind);
        }
    }

    /// Select the item of `kind`, if it's among the matches.
    pub fn select(&mut self, kind: &ItemKind) {
        let found = self.matches.iter().position(|m| {
            self.items
                .get(m.item)
                .is_some_and(|item| item.kind == *kind)
        });
        if let Some(index) = found {
            self.selected = index;
        }
        self.sync_preview();
    }

    /// At the Themes level, preview the selected theme; at a setting that
    /// changes how things look, the selected value (D59); elsewhere, none.
    fn sync_preview(&mut self) {
        let selected = self.selected_item().map(|item| &item.kind);
        self.preview = match (self.level(), selected) {
            (Level::Themes, Some(ItemKind::Theme(name))) => Some(Preview::Theme(name.clone())),
            (Level::Setting(setting), Some(ItemKind::Setting(choice))) if setting.previews() => {
                Some(Preview::Setting(*choice))
            }
            (_, Some(_) | None) => None,
        };
    }

    /// The theme being tried on.
    pub fn previewed_theme(&self) -> Option<&str> {
        match &self.preview {
            Some(Preview::Theme(name)) => Some(name),
            Some(Preview::Setting(_)) | None => None,
        }
    }

    /// The setting value being tried on.
    pub const fn previewed_setting(&self) -> Option<SettingChoice> {
        match &self.preview {
            Some(Preview::Setting(choice)) => Some(*choice),
            Some(Preview::Theme(_)) | None => None,
        }
    }

    /// The selected item, if it can be selected (never a header or a
    /// disabled item, so nothing acts on those).
    pub fn selected_item(&self) -> Option<&Item> {
        let found = self.matches.get(self.selected)?;
        self.items.get(found.item).filter(|item| item.selectable())
    }

    /// Match the query again; back to the best result.
    pub fn refilter(&mut self) {
        // Typing a nickname or searching messages: the query is the input, every item stays.
        if let Level::Nickname(_) | Level::Search(_) | Level::SettingText(_) = self.level() {
            self.matches = (0..self.items.len())
                .map(|item| Match {
                    item,
                    highlight: Vec::new(),
                })
                .collect();
            self.selected = if matches!(self.level(), Level::Search(_)) {
                self.items
                    .iter()
                    .position(|item| matches!(item.kind, ItemKind::SearchHit { .. }))
                    .or_else(|| {
                        self.items
                            .iter()
                            .position(|item| matches!(item.kind, ItemKind::SavedQuery(_)))
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            self.offset = 0;
            self.sync_preview();
            return;
        }
        let filter_text = if self.level() == Level::AttachFile {
            super::files::split_file_query(self.input.text()).1.trim()
        } else {
            self.input.text().trim()
        };
        let pattern = Pattern::parse(filter_text, CaseMatching::Smart, Normalization::Smart);
        let typed = !pattern.atoms.is_empty();
        let mut buf = Vec::new();
        let mut found: Vec<(Option<u32>, Vec<u32>)> = Vec::with_capacity(self.items.len());
        for item in &self.items {
            let mut highlight = Vec::new();
            let haystack = Utf32Str::new(&item.title, &mut buf);
            let score = pattern.indices(haystack, &mut self.matcher, &mut highlight);
            highlight.sort_unstable();
            highlight.dedup();
            let bonus = item.recent.map_or(0, |r| {
                RECENT_BONUS * u32::try_from(RECENT_MAX.saturating_sub(r)).unwrap_or(0)
            });
            let score = score.map(|score| if typed { score + bonus } else { 0 });
            found.push((score, highlight));
        }
        let (matches, best) = if self.items.iter().any(|item| item.kind == ItemKind::Header) {
            grouped(&self.items, found)
        } else {
            ranked(found)
        };
        self.matches = matches;
        self.selected = best;
        self.offset = 0;
        self.sync_preview();
    }

    /// Move the selection by `delta`, wrapping at the ends for single steps
    /// and stopping there for pages; keep it within `rows` of `offset`.
    /// Headers are skipped.
    pub fn step(&mut self, delta: isize, rows: usize) {
        let len = self.matches.len();
        if len == 0 {
            return;
        }
        let last = len - 1;
        let single = |from: usize, up: bool| match (up, from) {
            (true, 0) => last,
            (true, i) => i - 1,
            (false, i) if i == last => 0,
            (false, i) => i + 1,
        };
        let up = delta < 0;
        if delta.unsigned_abs() == 1 {
            let mut at = single(self.selected, up);
            for _ in 0..len {
                if self.selectable(at) {
                    break;
                }
                at = single(at, up);
            }
            self.selected = at;
        } else {
            let target = if up {
                self.selected.saturating_sub(delta.unsigned_abs())
            } else {
                self.selected.saturating_add(delta.unsigned_abs()).min(last)
            };
            // The nearest item that can be selected, onwards first.
            let onwards = (target..=last).find(|i| self.selectable(*i));
            let back = (0..target).rev().find(|i| self.selectable(*i));
            self.selected = onwards.or(back).unwrap_or(self.selected);
        }
        self.reveal(rows);
        self.sync_preview();
    }

    /// Whether the result at `index` can be selected.
    pub fn selectable(&self, index: usize) -> bool {
        self.matches
            .get(index)
            .and_then(|m| self.items.get(m.item))
            .is_some_and(Item::selectable)
    }

    /// Scroll so the selection is among the `rows` shown.
    pub fn reveal(&mut self, rows: usize) {
        let rows = rows.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + rows {
            self.offset = self.selected + 1 - rows;
        }
    }
}

/// Matches best first (ties in item order), and the best one's index.
fn ranked(found: Vec<(Option<u32>, Vec<u32>)>) -> (Vec<Match>, usize) {
    let mut found: Vec<(u32, usize, Vec<u32>)> = found
        .into_iter()
        .enumerate()
        .filter_map(|(item, (score, highlight))| Some((score?, item, highlight)))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let matches = found
        .into_iter()
        .map(|(_, item, highlight)| Match { item, highlight })
        .collect();
    (matches, 0)
}

/// Matches in item order under their headers: an item matches by its own
/// title or its header's (typing `notif` lists the notification settings),
/// and a header is kept only above a match. The best match is selected.
fn grouped(items: &[Item], found: Vec<(Option<u32>, Vec<u32>)>) -> (Vec<Match>, usize) {
    let mut matches = Vec::new();
    let mut best: Option<(u32, usize)> = None;
    // The header above, its score and highlight, and whether it's shown.
    let mut header: Option<(usize, Option<u32>, Vec<u32>, bool)> = None;
    for (i, ((score, highlight), item)) in found.into_iter().zip(items).enumerate() {
        if item.kind == ItemKind::Header {
            header = Some((i, score, highlight, false));
            continue;
        }
        let under = header.as_ref().and_then(|(_, score, _, _)| *score);
        let Some(score) = score.max(under) else {
            continue;
        };
        if let Some((index, _, highlight, shown)) = header.as_mut()
            && !*shown
        {
            *shown = true;
            matches.push(Match {
                item: *index,
                highlight: std::mem::take(highlight),
            });
        }
        if item.selectable() && best.is_none_or(|(top, _)| score > top) {
            best = Some((score, matches.len()));
        }
        matches.push(Match { item: i, highlight });
    }
    (matches, best.map_or(0, |(_, index)| index))
}

/// The items of `level`, in the order shown when nothing is typed.
pub fn items(state: &State, level: Level, recent: &[PaletteCommand]) -> Vec<Item> {
    match level {
        Level::Root => categories(state),
        Level::Chats => conversations(state),
        Level::Search(scope) => search_items(state, scope),
        Level::Themes => themes(state),
        Level::Settings => settings(state),
        Level::Setting(setting) => setting_values(state, setting),
        Level::Commands => commands_by_recency(state, recent),
        Level::Keys => keybindings(&state.keymap),
        Level::People => people(state),
        Level::Nickname(id) => nickname_actions(state, id),
        Level::SettingText(setting) => setting_text_actions(setting),
        Level::AttachFile => Vec::new(),
        Level::ChatLinks => chat_links_and_files(state),
        Level::ForwardTo(target) => forward_destinations(state, target),
        Level::ReleaseNotes => release_notes_items(state),
        Level::Devices => devices(state),
        Level::Doctor => doctor_items(state),
    }
}

/// `Level::Doctor`: a top action to copy the full report, followed by each
/// diagnostic section and its rows (`#47`).
fn doctor_items(state: &State) -> Vec<Item> {
    let mut items = vec![
        Item::new(
            ItemKind::DoctorRow,
            "Copy full diagnostic report to clipboard",
        )
        .detail("enter"),
    ];
    for section in super::doctor::sections(state) {
        items.push(Item::new(ItemKind::Header, section.title));
        for row in section.rows {
            items.push(Item::new(ItemKind::DoctorRow, row.title).detail(row.detail));
        }
    }
    items
}

/// `Level::ReleaseNotes`: the 2–3 item "What's new for you" digest touching
/// keys and settings the user uses, any cached release notes from
/// `updates.latest`, and the compiled-in offline release history.
fn release_notes_items(state: &State) -> Vec<Item> {
    let mut items = Vec::new();
    let digest = super::updates::digest_for_user(state);
    if !digest.is_empty() {
        let header = match state.updates.since_version {
            Some(since) => format!("What's new for you · since {since}"),
            None => "What's new for you".to_owned(),
        };
        items.push(Item::new(ItemKind::Header, header));
        for (summary, detail) in digest {
            items.push(Item::new(ItemKind::Key, summary).detail(detail));
        }
    }

    let url_hint = if state.env.over_ssh {
        "copy link"
    } else {
        "open"
    };

    if let Some(release) = state.update_available() {
        items.push(Item::new(
            ItemKind::Header,
            format!("faro {} · available", release.version),
        ));
        for line in super::updates::parse_markdown_notes(&release.notes) {
            items.push(Item::new(ItemKind::Key, line).detail(release.version.to_string()));
        }
        items.push(command(
            PaletteCommand::OpenReleaseUrl,
            format!("Open faro {} release page on GitHub", release.version),
            Some(url_hint.to_owned()),
        ));
    }

    for bundled in super::updates::BUNDLED_RELEASES {
        if state
            .update_available()
            .is_some_and(|r| r.version == bundled.version && !r.notes.trim().is_empty())
        {
            continue;
        }
        items.push(Item::new(
            ItemKind::Header,
            format!("faro {} · {}", bundled.version, bundled.date),
        ));
        for entry in bundled.entries {
            items.push(Item::new(ItemKind::Key, entry.summary).detail(entry.detail));
        }
    }

    if state.update_available().is_none() {
        items.push(command(
            PaletteCommand::OpenReleaseUrl,
            format!(
                "Open faro {} release page on GitHub",
                env!("CARGO_PKG_VERSION")
            ),
            Some(url_hint.to_owned()),
        ));
    }

    items
}

/// `Level::Devices`: linked devices on this account, with last-seen and created dates.
fn devices(state: &State) -> Vec<Item> {
    state
        .devices
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|device| {
            let last_seen = format_device_date(device.last_seen);
            let created = format_device_date(device.created_at);
            let detail = if device.is_current {
                format!("this device · last seen {last_seen} · linked {created} · enter to unlink")
            } else if device.id == 1 {
                format!("primary · last seen {last_seen} · linked {created}")
            } else {
                format!("last seen {last_seen} · linked {created}")
            };
            Item::new(ItemKind::Device(device.id), device.display_name())
                .current(device.is_current)
                .detail(detail)
        })
        .collect()
}

fn format_device_date(ts: faro_core::domain::Timestamp) -> String {
    i64::try_from(ts.as_millis())
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map_or_else(
            || "unknown".to_owned(),
            |t| {
                t.to_zoned(jiff::tz::TimeZone::UTC)
                    .strftime("%Y-%m-%d")
                    .to_string()
            },
        )
}

/// `Level::ChatLinks`: attachments and URLs shared in the open conversation,
/// newest first, grouped under `Files & media` and `Links` headers.
fn chat_links_and_files(state: &State) -> Vec<Item> {
    let Some(open) = state.open.as_ref() else {
        return Vec::new();
    };
    let mut media = Vec::new();
    let mut links = Vec::new();
    let mut seen_urls: Vec<String> = Vec::new();
    for message in open.messages.iter().rev().filter(|m| !m.deleted) {
        let sender = state.sender_name(message);
        let msg_ref = message.reference();
        for (index, att) in message.attachments.iter().enumerate() {
            let title = if att.is_voice_note() {
                "voice note".to_owned()
            } else {
                att.file_name
                    .clone()
                    .unwrap_or_else(|| att.content_type.clone())
            };
            let status_or_size = match &att.status {
                faro_core::domain::AttachmentStatus::Downloaded(_) => att
                    .size
                    .map(faro_core::domain::format_byte_size)
                    .or_else(|| att.duration_secs.map(faro_core::audio::format_duration))
                    .unwrap_or_else(|| "downloaded".to_owned()),
                faro_core::domain::AttachmentStatus::Remote => match att.size {
                    Some(size) => format!(
                        "{} · not downloaded",
                        faro_core::domain::format_byte_size(size)
                    ),
                    None => "not downloaded".to_owned(),
                },
                faro_core::domain::AttachmentStatus::Failed => "download failed".to_owned(),
            };
            media.push(
                Item::new(
                    ItemKind::ChatAttachment {
                        message: msg_ref,
                        index,
                    },
                    title,
                )
                .detail(format!("{sender} · {status_or_size}")),
            );
        }
        for url in super::update::selection::message_links(message) {
            if !seen_urls.contains(&url) {
                seen_urls.push(url.clone());
                let detail = message
                    .previews
                    .iter()
                    .find(|p| p.url == url)
                    .and_then(|p| p.title.as_deref())
                    .map_or_else(
                        || sender.clone(),
                        |title| format!("{} · {sender}", crate::ui::text::one_line(title)),
                    );
                links.push(
                    Item::new(
                        ItemKind::ChatLink {
                            message: msg_ref,
                            url: url.clone(),
                        },
                        url,
                    )
                    .detail(detail),
                );
            }
        }
    }
    let mut items = Vec::new();
    if !media.is_empty() {
        items.push(Item::new(ItemKind::Header, "Files & media"));
        items.extend(media);
    }
    if !links.is_empty() {
        items.push(Item::new(ItemKind::Header, "Links"));
        items.extend(links);
    }
    items
}

/// `Level::Search(scope)`: a toggle between the open chat and all
/// conversations, saved queries when the input is empty, and SQLite search
/// hits (`Palette::search_hits`).
fn search_items(state: &State, scope: Option<ConversationId>) -> Vec<Item> {
    let mut items = Vec::new();
    match (scope, &state.open) {
        (Some(_), _) => {
            items.push(Item::new(
                ItemKind::Level(Level::Search(None)),
                "Search all conversations…",
            ));
        }
        (None, Some(open)) => {
            let title = state.conversation_title(open.id);
            items.push(Item::new(
                ItemKind::Level(Level::Search(Some(open.id))),
                format!("Search only in {title}…"),
            ));
        }
        (None, None) => {}
    }
    let query_empty = state
        .palette
        .as_ref()
        .is_none_or(|p| p.query().trim().is_empty());
    if query_empty && !state.saved_searches.is_empty() {
        items.push(Item::new(ItemKind::Header, "Saved queries"));
        for (idx, saved) in state.saved_searches.iter().enumerate() {
            let detail = if idx < 9 {
                format!("alt+{} · ctrl+s unpin", idx + 1)
            } else {
                "ctrl+s unpin".to_owned()
            };
            items
                .push(Item::new(ItemKind::SavedQuery(saved.clone()), saved.clone()).detail(detail));
        }
    }
    let hits = state
        .palette
        .as_ref()
        .map_or(&[][..], |p| p.search_hits.as_slice());
    for hit in hits {
        if scope.is_none()
            && state.list.allowlist
            && !state
                .summary(hit.conversation)
                .is_some_and(|s| s.state.allowed || s.state.pinned.is_some())
        {
            continue;
        }
        let file_label = hit
            .message
            .attachments
            .iter()
            .find_map(|a| a.file_name.as_deref());
        let raw = match (
            state
                .readable_body(&hit.message)
                .or_else(|| hit.matched_transcript.clone()),
            file_label,
        ) {
            (Some(text), Some(name)) if !text.contains(name) => format!("{text} · {name}"),
            (Some(text), _) => text,
            (None, Some(name)) => name.to_owned(),
            (None, None) => hit
                .message
                .attachments
                .first()
                .map(|a| a.content_type.clone())
                .unwrap_or_default(),
        };
        let snippet = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let sender = state.sender_name(&hit.message);
        let detail = match scope {
            Some(_) => sender,
            None => format!("{} · {sender}", state.conversation_title(hit.conversation)),
        };
        items.push(
            Item::new(
                ItemKind::SearchHit {
                    conversation: hit.conversation,
                    message: hit.message.reference(),
                },
                snippet,
            )
            .detail(detail),
        );
    }
    items
}

/// Directory entries in `Level::AttachFile` as palette items (folders first, then files).
pub fn file_items(entries: &[FileEntry]) -> Vec<Item> {
    entries
        .iter()
        .map(|entry| {
            if entry.is_dir {
                Item::new(
                    ItemKind::EnterDir(entry.dir_query.clone()),
                    format!("{}/", entry.name),
                )
                .detail("folder")
            } else {
                Item::new(ItemKind::AttachFile(entry.path.clone()), entry.name.clone())
                    .detail(faro_core::domain::format_byte_size(entry.size))
            }
        })
        .collect()
}

/// Everyone faro knows, by name, to give one a nickname (D65).
fn people(state: &State) -> Vec<Item> {
    let mut items: Vec<Item> = state
        .people()
        .into_iter()
        .map(|id| {
            let detail = if state.nicknames.contains_key(&id) {
                "nickname"
            } else {
                ""
            };
            Item::new(ItemKind::Level(Level::Nickname(id)), state.person_name(id)).detail(detail)
        })
        .collect();
    items.sort_by_cached_key(|item| item.title.to_lowercase());
    items
}

/// At `Nickname`: save what's typed, or remove the nickname there is.
fn nickname_actions(state: &State, id: ServiceId) -> Vec<Item> {
    let mut items = vec![Item::new(ItemKind::SetNickname(id), "Save as nickname").detail("enter")];
    if state.nicknames.contains_key(&id) {
        items.push(Item::new(
            ItemKind::ClearNickname(id),
            "Remove the nickname",
        ));
    }
    items
}

/// The palette path that types a nickname for `id`.
pub fn nickname_path(id: ServiceId) -> Vec<Level> {
    vec![Level::Root, Level::People, Level::Nickname(id)]
}

/// The root: one item per category, with its prefix (or, for themes, the
/// theme in use) on the right.
fn categories(state: &State) -> Vec<Item> {
    let open = state.open.as_ref().map(|o| o.id);
    Level::categories(open)
        .into_iter()
        .map(|level| {
            let detail = match level {
                Level::Themes => current_theme(state).to_owned(),
                Level::Root
                | Level::Chats
                | Level::Search(_)
                | Level::Settings
                | Level::Setting(_)
                | Level::Commands
                | Level::Keys
                | Level::People
                | Level::Nickname(_)
                | Level::SettingText(_)
                | Level::AttachFile
                | Level::ChatLinks
                | Level::ForwardTo(_)
                | Level::ReleaseNotes
                | Level::Devices
                | Level::Doctor => level.prefix().map(String::from).unwrap_or_default(),
            };
            Item::new(ItemKind::Level(level), level.title()).detail(detail)
        })
        .collect()
}

/// Commands, the 3 most recently used first.
fn commands_by_recency(state: &State, recent: &[PaletteCommand]) -> Vec<Item> {
    let mut commands = commands(state);
    for item in &mut commands {
        if let ItemKind::Command(command) = item.kind {
            item.recent = recent.iter().position(|c| *c == command);
        }
    }
    let (mut first, rest): (Vec<Item>, Vec<Item>) = commands
        .into_iter()
        .partition(|item| item.recent.is_some_and(|r| r < RECENT_SHOWN));
    first.sort_by_key(|item| item.recent);
    first.extend(rest);
    first
}

fn command(command: PaletteCommand, title: impl Into<String>, detail: Option<String>) -> Item {
    Item::new(ItemKind::Command(command), title).detail(detail.unwrap_or_default())
}

fn commands(state: &State) -> Vec<Item> {
    let keymap = &state.keymap;
    let key = |action: Action, focus: Focus| keymap.hint(&[action], focus);
    let global = |action: Action, title: &str| {
        command(
            PaletteCommand::Action(action),
            title,
            key(action, Focus::Input),
        )
    };
    let mut items = vec![
        global(Action::NextConversation, "Next conversation"),
        global(Action::PreviousConversation, "Previous conversation"),
        command(
            PaletteCommand::SearchMessages,
            "Search messages…",
            key(Action::SearchMessages, Focus::Input),
        ),
    ];
    items.extend(open_conversation_commands(state));
    items.extend(selected_message_commands(state));
    let switch = Action::Curate(Curate::SwitchView);
    items.push(command(
        PaletteCommand::Action(switch),
        match state.list.view {
            SidebarView::Chats => "Show the archive",
            SidebarView::Archive => "Show the chats",
        },
        key(switch, Focus::Conversations),
    ));
    let mode = Action::Curate(Curate::AllowlistMode);
    items.push(command(
        PaletteCommand::Action(mode),
        if state.list.allowlist {
            "Turn allowlist mode off"
        } else {
            "Turn allowlist mode on"
        },
        key(mode, Focus::Conversations),
    ));
    for (i, (_, label)) in INACTIVE_CHOICES.iter().enumerate() {
        items.push(command(
            PaletteCommand::HideInactive(i),
            format!("Hide conversations inactive for {label}"),
            None,
        ));
    }
    items.push(Item::new(ItemKind::Level(Level::People), "Set a nickname…"));
    items.push(Item::new(
        ItemKind::Level(Level::Devices),
        "Linked devices…",
    ));
    items.push(Item::new(
        ItemKind::Level(Level::Doctor),
        "Doctor: terminal, graphics & config report…",
    ));
    items.push(global(
        Action::ToggleLeftSidebar,
        if state.panels.left {
            "Hide the conversation list"
        } else {
            "Show the conversation list"
        },
    ));
    items.push(global(
        Action::ToggleRightSidebar,
        if state.panels.right {
            "Hide conversation info"
        } else {
            "Show conversation info"
        },
    ));
    items.push(global(
        Action::ToggleBell,
        if state.config.notifications.bell {
            "Turn do not disturb on (bell off)"
        } else {
            "Turn do not disturb off (bell on)"
        },
    ));
    items.push(global(Action::Lock, "Lock screen"));
    items.push(command(
        PaletteCommand::ShowKeys,
        "Show keybindings",
        key(Action::Help, Focus::Input),
    ));
    let notes_version = state.update_available().map_or_else(
        || env!("CARGO_PKG_VERSION").to_owned(),
        |r| r.version.to_string(),
    );
    items.push(command(
        PaletteCommand::ReleaseNotes,
        format!("Release notes for faro {notes_version}"),
        None,
    ));
    items.push(command(PaletteCommand::About, "About faro", None));
    items.push(global(Action::Quit, "Quit"));
    items
}

/// The theme in use (an unknown name was reported when the config was read
/// and the default is drawn).
pub fn current_theme(state: &State) -> &str {
    Theme::find(&state.config.theme, &state.user_themes)
        .map_or(faro_core::config::DEFAULT_THEME, |theme| {
            theme.name.as_str()
        })
}

/// Every theme, the one in use marked and where it comes from on the right.
fn themes(state: &State) -> Vec<Item> {
    let current = current_theme(state);
    Theme::all(&state.user_themes)
        .into_iter()
        .map(|theme| {
            let name = theme.name.as_str();
            Item::new(ItemKind::Theme(name.to_owned()), name)
                .current(name == current)
                .detail(theme.source.label().unwrap_or_default())
        })
        .collect()
}

/// `Settings ›`: every setting under its section's header, its current
/// value on the right.
fn settings(state: &State) -> Vec<Item> {
    let mut items = Vec::new();
    for group in SettingsGroup::ALL {
        items.push(Item::new(ItemKind::Header, group.title()));
        items.extend(group.settings().iter().map(|s| setting_item(state, *s)));
    }
    items
}

/// The row for `setting`, showing its current value: flipped by `Enter`
/// if it's on/off, else opening its values.
fn setting_item(state: &State, setting: Setting) -> Item {
    let value = value_title(state, setting);
    let kind = if setting.is_switch() {
        ItemKind::Switch(setting)
    } else {
        ItemKind::Level(Level::Setting(setting))
    };
    if let Some(reason) = setting.unavailable(state) {
        return Item {
            disabled: true,
            ..Item::new(kind, setting.title()).detail(reason)
        };
    }
    let value = match (setting, state.transcriber_missing()) {
        (Setting::Transcribe, Some(program)) => format!("{value} · {program} not found"),
        (Setting::TranscribeCommand, Some(_)) => format!("{value} · not found"),
        _ => value,
    };
    Item {
        changed: setting.changed(state),
        ..Item::new(kind, setting.title()).detail(value)
    }
}

/// How the value of `setting` in use is shown (`on`, `compact`, …).
pub fn value_title(state: &State, setting: Setting) -> String {
    values(state, setting)
        .into_iter()
        .find(|(_, _, current)| *current)
        .map_or_else(|| typed_value(state, setting), |(_, title, _)| title)
}

/// A text setting's value as written, for one no preset matches (D145).
pub fn typed_value(state: &State, setting: Setting) -> String {
    match state.config.value(setting.key()) {
        Some(faro_core::config::ConfigValue::Text(text)) => text,
        Some(faro_core::config::ConfigValue::Uint(number)) => number.to_string(),
        Some(_) | None => String::new(),
    }
}

/// The value an on/off `setting` changes to: the one not in use.
pub fn flipped(state: &State, setting: Setting) -> Option<SettingChoice> {
    values(state, setting)
        .into_iter()
        .find(|(_, _, current)| !*current)
        .map(|(choice, _, _)| choice)
}

/// The next (`forward`) or previous value of `setting`, wrapping around.
pub fn cycled(state: &State, setting: Setting, forward: bool) -> Option<SettingChoice> {
    let list = values(state, setting);
    let len = list.len();
    if len == 0 {
        return None;
    }
    let current = list
        .iter()
        .position(|(_, _, current)| *current)
        .unwrap_or(0);
    let next = if forward {
        (current + 1) % len
    } else if current == 0 {
        len - 1
    } else {
        current - 1
    };
    list.into_iter().nth(next).map(|(choice, _, _)| choice)
}

pub(crate) fn inline_images_note(state: &State) -> String {
    let mode = state.config.media.inline_images;
    match mode {
        InlineImages::Kitty | InlineImages::Sixel | InlineImages::Iterm2 => {
            return format!(
                "Forced {}: compatible terminal + tmux passthrough; no mosh.",
                mode.name()
            );
        }
        InlineImages::Off => return "Inline images off; attachment details remain.".to_owned(),
        InlineImages::Auto | InlineImages::Halfblocks => {}
    }
    if state.colors() == ColorDepth::Ansi16 {
        return "Images off: 16 colours (Settings › Colours).".to_owned();
    }
    if mode == InlineImages::Halfblocks {
        return "Halfblocks: coloured text cells, not terminal graphics.".to_owned();
    }
    if state.colors() == ColorDepth::Indexed256 {
        return "Auto: halfblocks — 256 colours (Settings › Colours).".to_owned();
    }
    // What's in the way, named so the user can fix it (D137).
    match &state.env.graphics_blocker {
        Some(GraphicsBlocker::MoshClient(tty)) => {
            return format!(
                "Auto: halfblocks — tmux client {tty} is connected through mosh. \
                 For images, detach it: tmux detach-client -t {tty}"
            );
        }
        Some(GraphicsBlocker::OtherClient { tty, term }) => {
            return format!(
                "Auto: halfblocks — tmux client {tty} is another terminal ({term}). \
                 For images, detach it: tmux detach-client -t {tty}"
            );
        }
        Some(GraphicsBlocker::NoClient) => {
            return "Auto: halfblocks — no tmux client was attached when faro last looked."
                .to_owned();
        }
        Some(GraphicsBlocker::Mosh) => {
            return "Auto: halfblocks — faro runs under mosh, which can't carry images.".to_owned();
        }
        None => {}
    }
    if state.env.graphics == super::state::GraphicsSupport::HalfblocksOnly {
        return "Auto: halfblocks — mixed or detached tmux clients.".to_owned();
    }
    match state.env.image_protocol {
        None => "Auto: halfblocks — asking the terminal about images.".to_owned(),
        Some(InlineImages::Halfblocks | InlineImages::Auto | InlineImages::Off) => {
            "Auto: halfblocks — the terminal didn't offer a graphics protocol.".to_owned()
        }
        Some(mode @ (InlineImages::Kitty | InlineImages::Sixel | InlineImages::Iterm2)) => {
            format!("Auto: {} — detected at startup.", mode.name())
        }
    }
}

/// Inline images' values: `auto` says what it draws with now and why (D137).
fn inline_images_values(state: &State) -> Vec<(SettingChoice, String, bool)> {
    InlineImages::ALL
        .iter()
        .map(|&mode| {
            let title = match mode {
                InlineImages::Auto => format!("auto ({})", inline_images_auto(state)),
                InlineImages::Kitty
                | InlineImages::Sixel
                | InlineImages::Iterm2
                | InlineImages::Halfblocks
                | InlineImages::Off => mode.name().to_owned(),
            };
            (
                SettingChoice::InlineImages(mode),
                title,
                state.config.media.inline_images == mode,
            )
        })
        .collect()
}

/// What `auto` draws with now, and why, in a few words: the Inline images
/// row says it like Colours says "auto (truecolor detected)" (D137).
fn inline_images_auto(state: &State) -> String {
    if state.colors() == ColorDepth::Ansi16 {
        return "off: 16 colours".to_owned();
    }
    if state.colors() == ColorDepth::Indexed256 {
        return "halfblocks: 256 colours".to_owned();
    }
    let why = match &state.env.graphics_blocker {
        Some(GraphicsBlocker::MoshClient(_)) => Some("a mosh client is attached"),
        Some(GraphicsBlocker::OtherClient { .. }) => Some("another terminal is attached"),
        Some(GraphicsBlocker::NoClient) => Some("no client attached"),
        Some(GraphicsBlocker::Mosh) => Some("mosh"),
        None => None,
    };
    if let Some(why) = why {
        return format!("halfblocks: {why}");
    }
    match state.env.image_protocol {
        Some(mode @ (InlineImages::Kitty | InlineImages::Sixel | InlineImages::Iterm2))
            if state.env.graphics == super::state::GraphicsSupport::Direct =>
        {
            mode.name().to_owned()
        }
        Some(_) | None => "halfblocks".to_owned(),
    }
}

/// A line about `level` shown above its results: at Themes, how the
/// terminal's colours change what's drawn (D57).
pub fn note(state: &State, level: Level) -> Option<String> {
    if level == Level::Setting(Setting::InlineImages) {
        return Some(inline_images_note(state));
    }
    if let Some(note) = transcription_note(state, level) {
        return Some(note);
    }
    if let Some(note) = sidebar_width_note(level) {
        return Some(note);
    }
    if level == Level::Setting(Setting::AutoLock) {
        let msg = if state.lock.verifier.is_some() {
            "Locks the screen after inactivity (or press Alt+L to lock now)."
        } else {
            "Locks the screen after inactivity (press Alt+L once to set a session passphrase, or run `faro encrypt`)."
        };
        return Some(msg.to_owned());
    }
    if let Level::Nickname(id) = level {
        let theirs = state
            .contacts
            .get(&id)
            .and_then(faro_core::domain::Contact::display_name)
            .map_or_else(String::new, |name| format!(" (their name: {name})"));
        return Some(format!(
            "Type a nickname{theirs}; only faro shows it. Empty removes it."
        ));
    }
    if level == Level::AttachFile {
        return Some(
            "Type a path; enter or → opens a folder, enter on a file attaches it (≤ 100 MB)."
                .to_owned(),
        );
    }
    if level == Level::ChatLinks {
        let has_any = state.open.as_ref().is_some_and(|open| {
            open.messages.iter().filter(|m| !m.deleted).any(|m| {
                !m.attachments.is_empty() || !super::update::selection::message_links(m).is_empty()
            })
        });
        return (!has_any)
            .then(|| "No shared links or files in the loaded history of this chat.".to_owned());
    }
    if matches!(level, Level::ForwardTo(_)) {
        return Some("Choose a conversation to forward this message to.".to_owned());
    }
    if level == Level::ReleaseNotes {
        return Some(match state.update_available() {
            Some(release) => format!(
                "faro {} is available (you have {}). Type to filter notes or keys.",
                release.version,
                env!("CARGO_PKG_VERSION")
            ),
            None => format!(
                "faro {} · release notes and changes touching your keys & settings.",
                env!("CARGO_PKG_VERSION")
            ),
        });
    }
    if matches!(level, Level::Search(_)) {
        return Some(
            "Operators: from:name  in:chat  has:image|file|voice|link  before:YYYY-MM  after:YYYY-MM  is:unread · ctrl+s pin"
                .to_owned(),
        );
    }
    if level == Level::Devices {
        return Some(match &state.devices {
            None => "Fetching linked devices…".to_owned(),
            Some(devices) if devices.is_empty() => {
                "No linked devices reported by the backend.".to_owned()
            }
            Some(_) => {
                "Linked devices on this account. Enter on this device unlinks it; manage other devices on your phone."
                    .to_owned()
            }
        });
    }
    if level == Level::Doctor {
        return Some(
            "Terminal capabilities, graphics fallback, config conflicts & keymap clashes. Enter copies the full report."
                .to_owned(),
        );
    }
    if level != Level::Themes {
        return None;
    }
    match state.colors() {
        ColorDepth::Truecolor => None,
        ColorDepth::Indexed256 => {
            Some("256 colours: themes are approximated (truecolor: Settings › Colours)".to_owned())
        }
        ColorDepth::Ansi16 => Some(
            "16 colours: the system theme is drawn; others need 256 colours or more".to_owned(),
        ),
    }
}

/// What to know at a transcription setting (D145): above all, a missing
/// local command and how to install it.
/// How else the list's width changes, and what a typed one may be (D156).
fn sidebar_width_note(level: Level) -> Option<String> {
    match level {
        Level::Setting(Setting::SidebarWidth) => Some(
            "Or drag the list's right border with the mouse, or press Alt+, and Alt+.".to_owned(),
        ),
        Level::SettingText(Setting::SidebarWidth) => Some(format!(
            "Type a width from {SIDEBAR_WIDTH_MIN} to {SIDEBAR_WIDTH_MAX} columns."
        )),
        Level::Root
        | Level::Chats
        | Level::Search(_)
        | Level::Themes
        | Level::Settings
        | Level::Setting(_)
        | Level::Commands
        | Level::Keys
        | Level::People
        | Level::Nickname(_)
        | Level::SettingText(_)
        | Level::AttachFile
        | Level::ChatLinks
        | Level::ForwardTo(_)
        | Level::ReleaseNotes
        | Level::Devices
        | Level::Doctor => None,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "flat match over every Level and Setting variant for transcription notes"
)]
fn transcription_note(state: &State, level: Level) -> Option<String> {
    let missing = state.transcriber_missing().map(|program| {
        let install = match program {
            "whisper" => ": brew install openai-whisper (macOS) or pipx install openai-whisper",
            "whisper-cli" => ": brew install whisper-cpp (macOS) or build whisper.cpp",
            _ => " on your PATH",
        };
        format!("{program} isn't installed{install}")
    });
    let key_note = "The API key comes from $FARO_TRANSCRIBE_API_KEY, $OPENAI_API_KEY or the \
                    system keychain, never the config file. Each chat asks first.";
    let note = match level {
        Level::Setting(Setting::TranscribeCommand) => missing.unwrap_or_else(|| {
            "Custom… runs your own command with {file}, {model}, {language} and {out_dir} \
             filled in."
                .to_owned()
        }),
        Level::Setting(Setting::Transcribe) => missing.unwrap_or_else(|| {
            "Local runs Whisper on this computer; remote sends the audio to an API. t \
             transcribes the selected voice note."
                .to_owned()
        }),
        Level::Setting(Setting::TranscribeModel) => {
            if state.config.voice.program() == "whisper-cli" {
                "whisper.cpp needs a model file: Custom… and its path (e.g. \
                 ~/models/ggml-small.bin)."
                    .to_owned()
            } else {
                "All are multilingual; larger is more accurate but slower. The first use \
                 downloads the model."
                    .to_owned()
            }
        }
        Level::Setting(Setting::TranscribeEndpoint | Setting::TranscribeRemoteModel) => {
            key_note.to_owned()
        }
        Level::Setting(Setting::TranscribeLanguage) => {
            "A hint helps most with short notes; auto-detect guesses from the first seconds."
                .to_owned()
        }
        Level::Setting(Setting::TranscribeMaxSecs) => "Longer voice notes wait for t.".to_owned(),
        Level::SettingText(setting) => match setting {
            Setting::TranscribeModel => "Type a model name, or the path to a whisper.cpp \
                                         model file."
                .to_owned(),
            Setting::TranscribeRemoteModel => {
                "Type the model name the endpoint expects.".to_owned()
            }
            Setting::TranscribeLanguage => "Type a two-letter language code (e.g. ro); empty \
                                            detects the language."
                .to_owned(),
            Setting::TranscribeCommand => "Type a command; {file}, {model}, {language} and \
                                           {out_dir} are filled in."
                .to_owned(),
            Setting::TranscribeEndpoint => "Type the URL of an OpenAI-compatible \
                                            /audio/transcriptions endpoint."
                .to_owned(),
            Setting::Colors
            | Setting::Emoji
            | Setting::SidebarWidth
            | Setting::Layout
            | Setting::BubbleStyle
            | Setting::BubblePadding
            | Setting::MatchWidths
            | Setting::Density
            | Setting::Timestamps
            | Setting::ReduceMotion
            | Setting::Beam
            | Setting::Bell
            | Setting::Title
            | Setting::Desktop
            | Setting::Privacy
            | Setting::Allowlist
            | Setting::RestoreSession
            | Setting::Avatars
            | Setting::Previews
            | Setting::PreviewTime
            | Setting::PreviewActivity
            | Setting::InlineImages
            | Setting::AutoDownload
            | Setting::Transcribe
            | Setting::TranscribeAuto
            | Setting::TranscribeMaxSecs
            | Setting::Mouse
            | Setting::PasteFromSystem
            | Setting::AutoLock
            | Setting::UpdateCheck => return None,
        },
        Level::Root
        | Level::Chats
        | Level::Search(_)
        | Level::Themes
        | Level::Settings
        | Level::Setting(_)
        | Level::Commands
        | Level::Keys
        | Level::People
        | Level::Nickname(_)
        | Level::AttachFile
        | Level::ChatLinks
        | Level::ForwardTo(_)
        | Level::ReleaseNotes
        | Level::Devices
        | Level::Doctor => return None,
    };
    Some(note)
}

/// At `SettingText`: save what's typed (D145).
fn setting_text_actions(setting: Setting) -> Vec<Item> {
    vec![
        Item::new(
            ItemKind::SetText(setting),
            format!("Save as {}", setting.title().to_lowercase()),
        )
        .detail("enter"),
    ]
}

/// `setting`'s values as items, the one in use marked.
fn setting_values(state: &State, setting: Setting) -> Vec<Item> {
    let overridden = state.env.emoji.from_cli && setting == Setting::Emoji;
    let values = values(state, setting);
    let preset = values.iter().any(|(_, _, current)| *current);
    let mut items: Vec<Item> = values
        .into_iter()
        .map(|(choice, title, current)| {
            let item = Item::new(ItemKind::Setting(choice), title).current(current);
            if overridden {
                item.detail("--emoji decides for now")
            } else {
                item.detail(value_detail(state, choice))
            }
        })
        .collect();
    if setting.takes_text() {
        let typed = if preset {
            String::new()
        } else {
            typed_value(state, setting)
        };
        items.push(
            Item::new(ItemKind::Level(Level::SettingText(setting)), "Custom…")
                .current(!preset)
                .detail(typed),
        );
    }
    items
}

/// What a preset value means, shown beside it (D145).
fn value_detail(state: &State, choice: SettingChoice) -> &'static str {
    let lookup = |list: &[(&'static str, &'static str)], key: &str| {
        list.iter().find(|(k, _)| *k == key).map_or("", |(_, d)| *d)
    };
    match choice {
        SettingChoice::TranscribeModel(model) => lookup(LOCAL_MODELS, model),
        SettingChoice::TranscribeCommand(command) => lookup(TRANSCRIBE_COMMANDS, command),
        SettingChoice::TranscribeEndpoint(url) => TRANSCRIBE_ENDPOINTS
            .iter()
            .find(|(u, _, _)| *u == url)
            .map_or("", |(_, _, host)| *host),
        SettingChoice::TranscribeRemoteModel(model) => {
            lookup(remote_models(&state.config.voice.transcribe_url), model)
        }
        SettingChoice::TranscribeLanguage(code) => code,
        SettingChoice::Transcribe(mode) => match mode {
            TranscribeMode::Off => "t does nothing",
            TranscribeMode::Local => "Whisper on this computer",
            TranscribeMode::Remote => "an API you choose; each chat asks first",
        },
        SettingChoice::Beam(beam) => match beam {
            LogoBeam::Ambient => "colour follows connection state, density follows unread pressure",
            LogoBeam::Static => "fixed warm lamp colour and standard fan",
        },
        SettingChoice::Colors(_)
        | SettingChoice::Layout(_)
        | SettingChoice::BubbleStyle(_)
        | SettingChoice::BubblePadding(_)
        | SettingChoice::MatchWidths(_)
        | SettingChoice::Density(_)
        | SettingChoice::Timestamps(_)
        | SettingChoice::Allowlist(_)
        | SettingChoice::RestoreSession(_)
        | SettingChoice::Avatars(_)
        | SettingChoice::SidebarWidth(_)
        | SettingChoice::Previews(_)
        | SettingChoice::PreviewTime(_)
        | SettingChoice::PreviewActivity(_)
        | SettingChoice::Emoji(_)
        | SettingChoice::Desktop(_)
        | SettingChoice::Privacy(_)
        | SettingChoice::InlineImages(_)
        | SettingChoice::AutoDownload(_)
        | SettingChoice::TranscribeAuto(_)
        | SettingChoice::TranscribeMaxSecs(_)
        | SettingChoice::Mouse(_)
        | SettingChoice::PasteFromSystem(_)
        | SettingChoice::Bell(_)
        | SettingChoice::Title(_)
        | SettingChoice::ReduceMotion(_)
        | SettingChoice::AutoLock(_)
        | SettingChoice::UpdateCheck(_) => "",
    }
}

fn choice_values<C: faro_core::config::Choice + Copy + PartialEq>(
    all: &[C],
    current: C,
    make: fn(C) -> SettingChoice,
) -> Vec<(SettingChoice, String, bool)> {
    all.iter()
        .map(|&v| (make(v), v.name().into(), v == current))
        .collect()
}

/// Text presets as values: the name is the title (D145).
fn presets(
    list: &[(&'static str, &'static str)],
    current: &str,
    make: fn(&'static str) -> SettingChoice,
) -> Vec<(SettingChoice, String, bool)> {
    list.iter()
        .map(|&(name, _)| (make(name), name.to_owned(), name == current))
        .collect()
}

/// Every value of `setting`: what choosing it does, its name, and whether
/// it's the one in use.
fn values(state: &State, setting: Setting) -> Vec<(SettingChoice, String, bool)> {
    match setting {
        Setting::Colors => ColorChoice::ALL
            .iter()
            .map(|&c| {
                let title = match c {
                    ColorChoice::Auto => format!("auto ({} detected)", state.env.colors.name()),
                    ColorChoice::Truecolor => "truecolor".to_owned(),
                    ColorChoice::Colors256 => "256".to_owned(),
                    ColorChoice::Colors16 => "16 (system theme)".to_owned(),
                };
                (SettingChoice::Colors(c), title, state.config.ui.colors == c)
            })
            .collect(),
        Setting::Emoji => choice_values(
            EmojiChoice::ALL,
            state.config.ui.emoji,
            SettingChoice::Emoji,
        ),
        Setting::Beam => choice_values(LogoBeam::ALL, state.config.beam, SettingChoice::Beam),
        Setting::Layout => MessageLayout::ALL
            .iter()
            .map(|&l| {
                let name = match l {
                    MessageLayout::Bubbles => "bubbles",
                    MessageLayout::Irc => "IRC",
                };
                (SettingChoice::Layout(l), name.into(), state.layout == l)
            })
            .collect(),
        Setting::BubbleStyle => choice_values(
            BubbleStyle::ALL,
            state.config.messages.bubble_style,
            SettingChoice::BubbleStyle,
        ),
        Setting::Density => choice_values(
            MessageDensity::ALL,
            state.config.messages.density,
            SettingChoice::Density,
        ),
        Setting::Timestamps => choice_values(
            TimestampFormat::ALL,
            state.config.messages.timestamps,
            SettingChoice::Timestamps,
        ),
        Setting::Desktop => choice_values(
            DesktopNotifications::ALL,
            state.config.notifications.desktop,
            SettingChoice::Desktop,
        ),
        Setting::Privacy => choice_values(
            NotificationPrivacy::ALL,
            state.config.notifications.privacy,
            SettingChoice::Privacy,
        ),
        Setting::SidebarWidth => sidebar_widths(state),
        Setting::Avatars => avatar_values(state),
        Setting::InlineImages => inline_images_values(state),
        Setting::AutoDownload => auto_download_values(state),
        Setting::AutoLock => AUTO_LOCK_MINS
            .iter()
            .map(|&(mins, title)| {
                (
                    SettingChoice::AutoLock(mins),
                    title.to_owned(),
                    mins == state.config.security.auto_lock_mins,
                )
            })
            .collect(),
        Setting::Transcribe => choice_values(
            TranscribeMode::ALL,
            state.config.voice.transcribe,
            SettingChoice::Transcribe,
        ),
        Setting::TranscribeMaxSecs
        | Setting::TranscribeModel
        | Setting::TranscribeCommand
        | Setting::TranscribeRemoteModel
        | Setting::TranscribeEndpoint
        | Setting::TranscribeLanguage => transcription_values(state, setting),
        Setting::Allowlist
        | Setting::TranscribeAuto
        | Setting::ReduceMotion
        | Setting::BubblePadding
        | Setting::MatchWidths
        | Setting::Bell
        | Setting::Title
        | Setting::RestoreSession
        | Setting::Previews
        | Setting::PreviewTime
        | Setting::PreviewActivity
        | Setting::Mouse
        | Setting::PasteFromSystem
        | Setting::UpdateCheck => switch_values(state, setting),
    }
}

/// `off` and `initials` (D158).
fn avatar_values(state: &State) -> Vec<(SettingChoice, String, bool)> {
    choice_values(
        Avatars::ALL,
        state.config.conversations.avatars,
        SettingChoice::Avatars,
    )
}

fn auto_download_values(state: &State) -> Vec<(SettingChoice, String, bool)> {
    choice_values(
        AutoDownload::ALL,
        state.config.media.auto_download,
        SettingChoice::AutoDownload,
    )
}

/// The preset widths of the conversation list (D156).
fn sidebar_widths(state: &State) -> Vec<(SettingChoice, String, bool)> {
    SIDEBAR_WIDTHS
        .iter()
        .map(|&(width, title)| {
            (
                SettingChoice::SidebarWidth(width),
                title.to_owned(),
                width == state.config.ui.sidebar_width,
            )
        })
        .collect()
}

/// `on` and `off` for the plain on/off settings.
fn switch_values(state: &State, setting: Setting) -> Vec<(SettingChoice, String, bool)> {
    let config = &state.config;
    let (make, current): (fn(bool) -> SettingChoice, bool) = match setting {
        Setting::Allowlist => (SettingChoice::Allowlist, state.list.allowlist),
        Setting::TranscribeAuto => (SettingChoice::TranscribeAuto, config.voice.auto_transcribe),
        Setting::ReduceMotion => (SettingChoice::ReduceMotion, config.reduce_motion),
        Setting::BubblePadding => (SettingChoice::BubblePadding, config.messages.bubble_padding),
        Setting::MatchWidths => (SettingChoice::MatchWidths, config.messages.match_widths),
        Setting::Bell => (SettingChoice::Bell, config.notifications.bell),
        Setting::RestoreSession => (
            SettingChoice::RestoreSession,
            config.conversations.restore_session,
        ),
        Setting::Previews => (SettingChoice::Previews, config.conversations.previews),
        Setting::PreviewTime => (
            SettingChoice::PreviewTime,
            config.conversations.preview_time,
        ),
        Setting::PreviewActivity => (
            SettingChoice::PreviewActivity,
            config.conversations.preview_activity,
        ),
        Setting::Title => (SettingChoice::Title, config.notifications.title),
        Setting::Mouse => (SettingChoice::Mouse, config.mouse.enabled),
        Setting::PasteFromSystem => (
            SettingChoice::PasteFromSystem,
            config.media.paste_from_system,
        ),
        Setting::UpdateCheck => (SettingChoice::UpdateCheck, config.updates.check),
        Setting::Colors
        | Setting::Emoji
        | Setting::Beam
        | Setting::Layout
        | Setting::BubbleStyle
        | Setting::Density
        | Setting::Timestamps
        | Setting::Desktop
        | Setting::Privacy
        | Setting::Avatars
        | Setting::SidebarWidth
        | Setting::InlineImages
        | Setting::AutoDownload
        | Setting::Transcribe
        | Setting::TranscribeMaxSecs
        | Setting::TranscribeModel
        | Setting::TranscribeLanguage
        | Setting::TranscribeCommand
        | Setting::TranscribeEndpoint
        | Setting::TranscribeRemoteModel
        | Setting::AutoLock => return Vec::new(),
    };
    [true, false]
        .into_iter()
        .map(|on| {
            (
                make(on),
                if on { "on" } else { "off" }.into(),
                on == current,
            )
        })
        .collect()
}

/// The preset values of the transcription settings below the mode (D145).
fn transcription_values(state: &State, setting: Setting) -> Vec<(SettingChoice, String, bool)> {
    match setting {
        Setting::TranscribeMaxSecs => TRANSCRIBE_MAX_SECS
            .iter()
            .map(|&(secs, title)| {
                (
                    SettingChoice::TranscribeMaxSecs(secs),
                    title.to_owned(),
                    secs == state.config.voice.transcribe_max_secs,
                )
            })
            .collect(),
        Setting::TranscribeModel => presets(
            LOCAL_MODELS,
            &state.config.voice.transcribe_model,
            SettingChoice::TranscribeModel,
        ),
        Setting::TranscribeCommand => presets(
            TRANSCRIBE_COMMANDS,
            &state.config.voice.transcribe_command,
            SettingChoice::TranscribeCommand,
        ),
        Setting::TranscribeRemoteModel => presets(
            remote_models(&state.config.voice.transcribe_url),
            &state.config.voice.transcribe_remote_model,
            SettingChoice::TranscribeRemoteModel,
        ),
        Setting::TranscribeEndpoint => TRANSCRIBE_ENDPOINTS
            .iter()
            .map(|&(url, name, _)| {
                (
                    SettingChoice::TranscribeEndpoint(url),
                    name.to_owned(),
                    url == state.config.voice.transcribe_url,
                )
            })
            .collect(),
        Setting::TranscribeLanguage => TRANSCRIBE_LANGUAGES
            .iter()
            .map(|&(code, name)| {
                (
                    SettingChoice::TranscribeLanguage(code),
                    name.to_owned(),
                    code == state.config.voice.transcribe_language,
                )
            })
            .collect(),
        Setting::Colors
        | Setting::Emoji
        | Setting::Beam
        | Setting::Layout
        | Setting::BubbleStyle
        | Setting::BubblePadding
        | Setting::MatchWidths
        | Setting::Density
        | Setting::Timestamps
        | Setting::ReduceMotion
        | Setting::Bell
        | Setting::Title
        | Setting::Desktop
        | Setting::Privacy
        | Setting::Allowlist
        | Setting::RestoreSession
        | Setting::Avatars
        | Setting::SidebarWidth
        | Setting::Previews
        | Setting::PreviewTime
        | Setting::PreviewActivity
        | Setting::InlineImages
        | Setting::AutoDownload
        | Setting::Transcribe
        | Setting::TranscribeAuto
        | Setting::Mouse
        | Setting::PasteFromSystem
        | Setting::AutoLock
        | Setting::UpdateCheck => Vec::new(),
    }
}

/// Curating the open conversation (the sidebar's highlight), with its name
/// as the path.
fn open_conversation_commands(state: &State) -> Vec<Item> {
    let Some(summary) = state.open.as_ref().and_then(|o| state.summary(o.id)) else {
        return vec![];
    };
    let current = &summary.state;
    let item = |action, title: &str, curate| {
        let hint = state
            .keymap
            .hint(&[Action::Curate(curate)], Focus::Conversations);
        command(PaletteCommand::Conversation(action), title, hint)
    };
    let mut items = vec![
        Item::new(ItemKind::Level(Level::AttachFile), "Send a file…"),
        command(
            PaletteCommand::Action(Action::PasteImage),
            "Paste an image from the clipboard",
            state.keymap.hint(&[Action::PasteImage], Focus::Input),
        ),
        Item::new(
            ItemKind::Level(Level::ChatLinks),
            "Links & files in this chat…",
        ),
        command(PaletteCommand::MediaGallery, "Media gallery", None),
    ];
    request_and_block_commands(state, current, &mut items);
    if state.open.as_ref().and_then(|o| o.unread_marker).is_some() {
        items.push(command(
            PaletteCommand::JumpUnread,
            "Jump to unread",
            state.keymap.hint(&[Action::JumpUnread], Focus::Messages),
        ));
    }
    pin_and_mute_commands(state, current, &mut items);
    let mentions = super::update::mentions_toggle(current);
    let toggle = |on, t, f, curate| {
        let (action, label) = if on { t } else { f };
        item(action, label, curate)
    };
    items.extend([
        item(mentions, mentions.label().0, Curate::Mentions),
        toggle(
            current.hidden,
            (ConversationAction::Unhide, "Unhide"),
            (ConversationAction::Hide, "Hide"),
            Curate::Hide,
        ),
        toggle(
            current.archived,
            (ConversationAction::Unarchive, "Unarchive"),
            (ConversationAction::Archive, "Archive"),
            Curate::Archive,
        ),
        toggle(
            current.allowed,
            (ConversationAction::Disallow, "Remove from the allowlist"),
            (ConversationAction::Allow, "Add to the allowlist"),
            Curate::Allow,
        ),
    ]);
    if state.config.voice.transcribe == TranscribeMode::Remote
        || current.remote_transcription == RemoteTranscription::Allowed
    {
        let remote_action = if current.remote_transcription == RemoteTranscription::Allowed {
            ConversationAction::DisallowRemoteTranscription
        } else {
            ConversationAction::AllowRemoteTranscription
        };
        items.push(command(
            PaletteCommand::Conversation(remote_action),
            remote_action.label().0,
            None,
        ));
    }
    if let ConversationId::Direct(peer) = summary.id
        && peer != state.me
    {
        items.push(Item::new(
            ItemKind::Level(Level::Nickname(peer)),
            "Set a nickname…",
        ));
        items.push(command(
            PaletteCommand::Conversation(ConversationAction::DisappearingTimer),
            ConversationAction::DisappearingTimer.label().0,
            None,
        ));
        if !current.request && !current.blocked {
            items.push(item(ConversationAction::Block, "Block", Curate::Block));
        }
    }
    if has_safety_number_command(state, summary.id) {
        items.push(command(
            PaletteCommand::Conversation(ConversationAction::SafetyNumber),
            ConversationAction::SafetyNumber.label().0,
            None,
        ));
    }
    let name = state.conversation_title(summary.id);
    for item in &mut items {
        item.path = Some(name.clone());
    }
    items
}

fn has_safety_number_command(state: &State, id: ConversationId) -> bool {
    match id {
        ConversationId::Direct(peer) => {
            peer != state.me
                && state
                    .identities
                    .get(&peer)
                    .and_then(|i| i.safety_number.as_ref())
                    .is_some()
        }
        ConversationId::Group(_) => !state.changed_identities(id).is_empty(),
    }
}

fn request_and_block_commands(
    state: &State,
    current: &faro_core::store::ConversationState,
    items: &mut Vec<Item>,
) {
    let item = |action, title: &str, curate| {
        let hint = state
            .keymap
            .hint(&[Action::Curate(curate)], Focus::Conversations);
        command(PaletteCommand::Conversation(action), title, hint)
    };
    if current.request {
        items.extend([
            item(
                ConversationAction::AcceptRequest,
                "Accept request",
                Curate::AcceptRequest,
            ),
            item(
                ConversationAction::DeleteRequest,
                "Delete conversation",
                Curate::DeleteRequest,
            ),
            item(ConversationAction::Block, "Block", Curate::Block),
            item(
                ConversationAction::ReportSpam,
                "Block & report spam",
                Curate::ReportSpam,
            ),
        ]);
    } else if current.blocked {
        items.extend([
            item(ConversationAction::Unblock, "Unblock", Curate::Block),
            item(
                ConversationAction::DeleteRequest,
                "Delete conversation",
                Curate::DeleteRequest,
            ),
        ]);
    }
}

fn pin_and_mute_commands(
    state: &State,
    current: &faro_core::store::ConversationState,
    items: &mut Vec<Item>,
) {
    let item = |action, title: &str, curate| {
        let hint = state
            .keymap
            .hint(&[Action::Curate(curate)], Focus::Conversations);
        command(PaletteCommand::Conversation(action), title, hint)
    };
    if current.pinned.is_some() {
        items.push(item(ConversationAction::Unpin, "Unpin", Curate::Pin));
        if state
            .conversations
            .iter()
            .filter(|c| c.state.pinned.is_some())
            .count()
            > 1
        {
            items.extend([
                item(ConversationAction::MoveUp, "Move pin up", Curate::MoveUp),
                item(
                    ConversationAction::MoveDown,
                    "Move pin down",
                    Curate::MoveDown,
                ),
            ]);
        }
    } else {
        items.push(item(ConversationAction::Pin, "Pin", Curate::Pin));
    }
    if current.muted_at(super::update::wall_clock()) {
        items.push(item(ConversationAction::Unmute, "Unmute", Curate::Mute));
    } else {
        items.extend(MUTE_CHOICES.iter().enumerate().map(|(i, (label, length))| {
            let title = if length.is_some() {
                format!("Mute for {label}")
            } else {
                "Mute until unmuted".to_owned()
            };
            command(PaletteCommand::MuteFor(i), title, None)
        }));
    }
}

/// What the selected message offers, if one is selected.
fn selected_message_commands(state: &State) -> Vec<Item> {
    let Some(open) = state.open.as_ref() else {
        return vec![];
    };
    let Some(selection) = open.selected else {
        return vec![];
    };
    let selected_msg = open
        .messages
        .iter()
        .rfind(|m| m.reference() == selection.message)
        .filter(|m| !m.deleted);
    let has_attachments = selected_msg.is_some_and(|m| !m.attachments.is_empty());
    let has_voice_note = selected_msg.is_some_and(|m| {
        m.attachments
            .iter()
            .any(faro_core::domain::AttachmentRef::is_voice_note)
    });
    let msg_item = |action: Action, title: &str| {
        let mut item = command(
            PaletteCommand::Action(action),
            title,
            state.keymap.hint(&[action], Focus::Messages),
        );
        item.path = Some("Message".to_owned());
        item
    };
    let mut items: Vec<Item> = [
        (Action::Reply, "Reply"),
        (Action::ForwardMessage, "Forward message"),
        (Action::ReactPicker, "React"),
        (Action::Copy, "Copy text"),
        (Action::MessageInfo, "Message info"),
        (Action::ViewThread, "View quote thread"),
        (Action::OpenLink, "Open link"),
    ]
    .into_iter()
    .map(|(action, title)| msg_item(action, title))
    .collect();
    if has_voice_note {
        for (action, title) in [
            (Action::PlayVoiceNote, "Play or stop voice note"),
            (Action::TranscribeVoiceNote, "Transcribe voice note"),
        ] {
            items.push(msg_item(action, title));
        }
    }
    if has_attachments {
        for (action, title) in [
            (Action::DownloadAttachment, "Download attachment"),
            (Action::OpenAttachment, "View or open attachment"),
            (Action::SaveAttachment, "Save attachment to Downloads"),
        ] {
            items.push(msg_item(action, title));
        }
        let mut copy_path = command(
            PaletteCommand::CopyAttachmentPath,
            "Copy attachment path",
            None,
        );
        copy_path.path = Some("Message".to_owned());
        items.push(copy_path);
    }
    if selected_msg
        .is_some_and(|m| matches!(m.direction, faro_core::domain::Direction::Outgoing(_)))
    {
        if state
            .capabilities
            .supports(faro_core::backend::Capability::Edit)
        {
            items.push(msg_item(Action::EditMessage, "Edit message"));
        }
        if state
            .capabilities
            .supports(faro_core::backend::Capability::DeleteForEveryone)
        {
            items.push(msg_item(Action::DeleteMessage, "Delete for everyone"));
        }
    }
    items
}

/// Every allowed conversation as a forward destination for `target`, in list
/// order.
fn forward_destinations(state: &State, target: MessageRef) -> Vec<Item> {
    conversations(state)
        .into_iter()
        .filter_map(|mut item| {
            if let ItemKind::Conversation(destination) = item.kind {
                if state.summary(destination).is_some_and(|s| {
                    matches!(state.section(s), Section::NotAllowed | Section::Blocked)
                }) {
                    return None;
                }
                item.kind = ItemKind::ForwardTo {
                    target,
                    destination,
                };
                Some(item)
            } else {
                None
            }
        })
        .collect()
}

/// Every conversation, including hidden, archived and empty ones, in list
/// order.
fn conversations(state: &State) -> Vec<Item> {
    let now = super::update::wall_clock();
    let open = state.open.as_ref().map(|o| o.id);
    state
        .conversations
        .iter()
        .map(|summary| {
            let mut notes = Vec::new();
            if summary.unread > 0 {
                notes.push(format!("{} unread", summary.unread));
            }
            if summary.state.muted_at(now) {
                notes.push("muted".to_owned());
            }
            if summary.state.notify == NotifyLevel::Mentions {
                notes.push("mentions only".to_owned());
            }
            match state.section(summary) {
                Section::Pinned => notes.push("pinned".to_owned()),
                Section::Recent => {}
                Section::Request => notes.push("message request".to_owned()),
                Section::Archived => notes.push("archived".to_owned()),
                Section::Hidden => notes.push("hidden".to_owned()),
                Section::NotAllowed => notes.push("not on the allowlist".to_owned()),
                Section::Blocked => notes.push("blocked".to_owned()),
            }
            if open == Some(summary.id) {
                notes.push("open".to_owned());
            }
            Item::new(
                ItemKind::Conversation(summary.id),
                state.conversation_title(summary.id),
            )
            .detail(notes.join(" · "))
        })
        .collect()
}

/// The keybinding list: every bound action per context, then popups.
fn keybindings(keymap: &Keymap) -> Vec<Item> {
    let mut items = Vec::new();
    for context in Context::ALL {
        let path = match context {
            Context::Global => "Everywhere",
            Context::Input => "Input",
            Context::Conversations => "Sidebar",
            Context::Messages => "Messages",
        };
        for action in Action::bindable() {
            let keys = keymap.keys_in(context, action);
            let (title, detail) = if let Action::JumpToPinned(n) = action {
                // One line for Alt+1…Alt+9.
                let last = keymap.keys_in(context, Action::JumpToPinned(9));
                let (1, Some(first), Some(last)) = (n, keys.first(), last.first()) else {
                    continue;
                };
                (
                    "Open pinned conversation 1–9".to_owned(),
                    format!("{}…{}", first.label(), last.label()),
                )
            } else if keys.is_empty() {
                continue;
            } else {
                (
                    action.title().unwrap_or_default().to_owned(),
                    keys.iter()
                        .map(|k| k.label())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            };
            let mut item = Item::new(ItemKind::Key, title).detail(detail);
            item.path = Some(path.to_owned());
            items.push(item);
        }
    }
    for (title, detail) in [
        ("Move", "↑↓, j k, tab"),
        ("Choose", "enter, or the shortcut shown"),
        ("Close", "esc, q"),
    ] {
        let mut item = Item::new(ItemKind::Key, title).detail(detail);
        item.path = Some("Menus".to_owned());
        items.push(item);
    }
    let palette = [
        ("Open a category", "enter, →"),
        ("Run a command, apply a setting", "enter"),
        ("Back up a level", "←, backspace"),
        ("Jump to chats, settings, commands, keys", "@ : > ?"),
    ];
    for (title, detail) in palette {
        let mut item = Item::new(ItemKind::Key, title).detail(detail);
        item.path = Some("Palette".to_owned());
        items.push(item);
    }
    items
}
