//! Update checks (D148): opt-in (`[updates] check`), notify only. This is
//! what's known and decided; the binary asks GitHub
//! (`Effect::CheckForUpdate`) and the reducer (`update/updates.rs`) decides
//! when to ask and what to say.

use std::fmt;
use std::time::Duration;

use faro_core::config::{Avatars, InlineImages, TranscribeMode};
use faro_core::domain::VerificationState;
use faro_core::rt::Instant;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::app::keymap::{Action, Keymap};
use crate::app::palette::{PaletteCommand, Setting, value_title};
use crate::app::state::{Focus, State};
use crate::ui::text::width;

/// Ask again this long after a check (and at most this often across runs).
pub const CHECK_INTERVAL: Duration = Duration::from_hours(12);
/// After a failed check (offline, rate-limited), try again this much later.
pub const RETRY_AFTER: Duration = Duration::from_hours(1);
/// A release is announced only once it's this old: time for the release
/// workflow to finish its uploads (`.deb`s, the Homebrew tap) and the AUR to
/// follow, and to pull a broken release before anyone is told about it.
pub const GRACE_SECS: i64 = 60 * 60;
/// How long the "faro x.y.z is available" notice stays in the footer.
pub const NOTICE_TIME: Duration = Duration::from_secs(10);

/// The repository releases come from (`https://github.com/<owner>/<repo>`).
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// A release version: `major.minor.patch`, nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// `1.2.3` or `v1.2.3`. Pre-releases (`1.2.3-dev`), build metadata and
    /// anything else are `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let mut number = || -> Option<u64> {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let version = Self {
            major: number()?,
            minor: number()?,
            patch: number()?,
        };
        parts.next().is_none().then_some(version)
    }

    /// This build's version; `None` for a pre-release (`0.2.0-dev`), which
    /// never checks.
    pub fn current() -> Option<Self> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The latest release on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// When it was published, in seconds since the Unix epoch.
    pub published_at: i64,
    /// Markdown release notes from the tag's `body` (empty when unknown).
    pub notes: String,
}

impl Release {
    pub const fn new(version: Version, published_at: i64) -> Self {
        Self {
            version,
            published_at,
            notes: String::new(),
        }
    }

    pub fn with_notes(version: Version, published_at: i64, notes: impl Into<String>) -> Self {
        Self {
            version,
            published_at,
            notes: notes.into(),
        }
    }

    /// Its release notes URL on GitHub.
    pub fn url(&self) -> String {
        Self::url_for(self.version)
    }

    /// The GitHub tag URL for `version`.
    pub fn url_for(version: Version) -> String {
        format!("{REPOSITORY}/releases/tag/v{version}")
    }

    /// As kept in `app_state` (`updates.latest`): `"<version> <published_at>"`
    /// on the first line, followed by `\n<notes>` when cached tag notes exist.
    pub fn to_stored(&self) -> String {
        let notes = self.notes.trim();
        if notes.is_empty() {
            format!("{} {}", self.version, self.published_at)
        } else {
            format!("{} {}\n{notes}", self.version, self.published_at)
        }
    }

    pub fn from_stored(text: &str) -> Option<Self> {
        let (first_line, notes) = text
            .split_once('\n')
            .map_or((text, ""), |(head, body)| (head, body.trim()));
        let (version, published_at) = first_line.split_once(' ')?;
        Some(Self {
            version: Version::parse(version)?,
            published_at: published_at.trim().parse().ok()?,
            notes: notes.to_owned(),
        })
    }

    /// Newer than `current` and old enough to announce at `now` (seconds).
    pub fn announceable(&self, current: Version, now: i64) -> bool {
        self.version > current && now >= self.published_at.saturating_add(GRACE_SECS)
    }
}

/// Why update checks can't run here, if they can't (the palette dims the
/// setting with this reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// Built without the `update-check` feature (a distro package), or the
    /// web demo.
    NotBuilt,
    /// `faro demo` (D87).
    Demo,
    /// A pre-release build (`-dev`): there's nothing to compare with.
    Development,
}

impl Unavailable {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::NotBuilt => "not in this build",
            Self::Demo => "not in the demo",
            Self::Development => "development build",
        }
    }
}

/// What the app knows about updates (`State::updates`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Updates {
    /// This binary can ask GitHub (built with `update-check`). The web demo
    /// and tests leave it off.
    pub built: bool,
    /// The newest release seen: from the last check, or remembered from the
    /// previous run.
    pub latest: Option<Release>,
    /// The release to tell the user about (newer, past the grace period),
    /// decided by the reducer so drawing needs no clock.
    pub available: Option<Release>,
    /// Already announced in this run, so it isn't repeated.
    pub announced: Option<Version>,
    /// When to ask next in this run; `None` while off, before the stored
    /// state is read, or while a check is in flight.
    pub next_check: Option<Instant>,
    /// Waiting for the stored state or for GitHub's answer.
    pub busy: bool,
    /// Previous version seen in `updates.latest` before an upgrade, used to
    /// scope the "what's new since you" digest.
    pub since_version: Option<Version>,
}

/// Maximum number of personalized items in the "What's new for you" digest.
pub const DIGEST_MAX: usize = 3;
/// Maximum display width (in terminal cells) of a wrapped release note row so
/// it renders without truncation inside an 80×24 palette (`box_width = 76`).
pub const NOTE_LINE_MAX_WIDTH: usize = 54;

/// Which user-configurable setting or keybinding a release change touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeTopic {
    Setting(Setting),
    Action(Action, Focus),
    Nicknames,
    DisappearingMessages,
    SafetyNumbers,
    General,
}

/// One change entry in the bundled offline release notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteEntry {
    pub summary: &'static str,
    pub detail: &'static str,
    pub topic: ChangeTopic,
}

/// Offline release notes for a shipped version of faro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundledRelease {
    pub version: Version,
    pub date: &'static str,
    pub entries: &'static [NoteEntry],
}

const fn ver(major: u64, minor: u64, patch: u64) -> Version {
    Version {
        major,
        minor,
        patch,
    }
}

/// Shipped release notes compiled into the binary for offline reading in the
/// palette (`Ctrl+P › Commands › Release notes`).
pub const BUNDLED_RELEASES: &[BundledRelease] = &[
    BundledRelease {
        version: ver(0, 1, 10),
        date: "2026-11-05",
        entries: &[
            NoteEntry {
                summary: "Quote thread lens popup for reply chains",
                detail: "T, t",
                topic: ChangeTopic::Action(Action::ViewThread, Focus::Messages),
            },
            NoteEntry {
                summary: "Disappearing messages expire locally with ◷ timers",
                detail: "◷ timer",
                topic: ChangeTopic::DisappearingMessages,
            },
            NoteEntry {
                summary: "Safety number verification and key-change send guard",
                detail: "alt+i",
                topic: ChangeTopic::SafetyNumbers,
            },
            NoteEntry {
                summary: "System lines for group changes, timers, and calls",
                detail: "layout",
                topic: ChangeTopic::Setting(Setting::Layout),
            },
            NoteEntry {
                summary: "Profile & group pictures in the list (avatars = auto)",
                detail: "avatars",
                topic: ChangeTopic::Setting(Setting::Avatars),
            },
            NoteEntry {
                summary: "Two-row message previews enabled by default",
                detail: "previews",
                topic: ChangeTopic::Setting(Setting::Previews),
            },
        ],
    },
    BundledRelease {
        version: ver(0, 1, 9),
        date: "2026-11-05",
        entries: &[NoteEntry {
            summary: "Send uploadLength when uploading files and images",
            detail: "ctrl+v",
            topic: ChangeTopic::Action(Action::PasteImage, Focus::Input),
        }],
    },
    BundledRelease {
        version: ver(0, 1, 8),
        date: "2026-11-03",
        entries: &[
            NoteEntry {
                summary: "Resizable conversation list via drag or keys",
                detail: "alt+,, alt+.",
                topic: ChangeTopic::Setting(Setting::SidebarWidth),
            },
            NoteEntry {
                summary: "Message previews, timestamps, and typing in the list",
                detail: "previews",
                topic: ChangeTopic::Setting(Setting::Previews),
            },
            NoteEntry {
                summary: "Initials avatar badges in the conversation list",
                detail: "avatars",
                topic: ChangeTopic::Setting(Setting::Avatars),
            },
            NoteEntry {
                summary: "Word wrapping in the message input box",
                detail: "input",
                topic: ChangeTopic::General,
            },
            NoteEntry {
                summary: "Keep chat focus when a late search window finishes",
                detail: "ctrl+f",
                topic: ChangeTopic::Action(Action::SearchMessages, Focus::Input),
            },
        ],
    },
    BundledRelease {
        version: ver(0, 1, 7),
        date: "2026-10-23",
        entries: &[NoteEntry {
            summary: "Survive malformed contact syncs; clean panic shutdown",
            detail: "sync",
            topic: ChangeTopic::General,
        }],
    },
    BundledRelease {
        version: ver(0, 1, 6),
        date: "2026-10-23",
        entries: &[
            NoteEntry {
                summary: "Paste clipboard images in Ghostty and system clipboard",
                detail: "ctrl+v",
                topic: ChangeTopic::Setting(Setting::PasteFromSystem),
            },
            NoteEntry {
                summary: "Convert pasted TIFF clipboard images to PNG",
                detail: "paste",
                topic: ChangeTopic::Action(Action::PasteImage, Focus::Input),
            },
        ],
    },
    BundledRelease {
        version: ver(0, 1, 5),
        date: "2026-10-23",
        entries: &[NoteEntry {
            summary: "TLS/crypto security updates and release attestations",
            detail: "security",
            topic: ChangeTopic::General,
        }],
    },
    BundledRelease {
        version: ver(0, 1, 4),
        date: "2026-10-22",
        entries: &[NoteEntry {
            summary: "Opt-in GitHub release update checks with 1h grace",
            detail: "updates.check",
            topic: ChangeTopic::Setting(Setting::UpdateCheck),
        }],
    },
    BundledRelease {
        version: ver(0, 1, 3),
        date: "2026-10-21",
        entries: &[NoteEntry {
            summary: "Drop forged sync messages from other accounts",
            detail: "security",
            topic: ChangeTopic::General,
        }],
    },
    BundledRelease {
        version: ver(0, 1, 2),
        date: "2026-10-19",
        entries: &[
            NoteEntry {
                summary: "Voice-note transcription settings and presets",
                detail: "t",
                topic: ChangeTopic::Setting(Setting::Transcribe),
            },
            NoteEntry {
                summary: "Auto-transcribe switch and length limit for voice notes",
                detail: "voice",
                topic: ChangeTopic::Setting(Setting::TranscribeAuto),
            },
        ],
    },
    BundledRelease {
        version: ver(0, 1, 1),
        date: "2026-10-14",
        entries: &[NoteEntry {
            summary: "Inline images name blocking tmux client and re-probe",
            detail: "inline_images",
            topic: ChangeTopic::Setting(Setting::InlineImages),
        }],
    },
    BundledRelease {
        version: ver(0, 1, 0),
        date: "2026-10-13",
        entries: &[
            NoteEntry {
                summary: "First public release: chats, palette, themes, nicknames",
                detail: "ctrl+p",
                topic: ChangeTopic::Nicknames,
            },
            NoteEntry {
                summary: "Allowlist mode and conversation curation shortcuts",
                detail: "allowlist",
                topic: ChangeTopic::Setting(Setting::Allowlist),
            },
        ],
    },
];

/// Whether `version` has compiled-in offline release notes.
pub fn has_bundled_notes(version: Version) -> bool {
    BUNDLED_RELEASES.iter().any(|r| r.version == version)
}

/// Clean and word-wrap raw GitHub release markdown (`Release::notes`) into
/// single-line palette rows that fit legibly in an 80×24 terminal.
pub fn parse_markdown_notes(raw: &str) -> Vec<String> {
    let mut rows = Vec::new();
    for raw_line in raw.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with("```") || trimmed.starts_with("---") {
            continue;
        }
        // Skip markdown headings that merely repeat the version or "What's Changed".
        if let Some(heading) = trimmed.strip_prefix('#') {
            let heading = heading.trim_start_matches('#').trim();
            let lower = heading.to_ascii_lowercase();
            if heading.is_empty()
                || lower.starts_with("what's changed")
                || lower.starts_with("whats changed")
                || lower.starts_with("faro ")
                || lower.starts_with("v0.")
                || lower.starts_with("0.")
                || lower == "release notes"
                || lower == "changelog"
            {
                continue;
            }
        }
        let stripped = trimmed
            .trim_start_matches('#')
            .trim()
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
            .or_else(|| trimmed.strip_prefix("• "))
            .unwrap_or(trimmed.trim_start_matches('#').trim());
        let cleaned = strip_inline_markdown(stripped);
        let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        if cleaned.is_empty() {
            continue;
        }
        let wrapped = wrap_note_line(&cleaned, NOTE_LINE_MAX_WIDTH);
        for (i, part) in wrapped.into_iter().enumerate() {
            if i == 0 {
                rows.push(part);
            } else {
                rows.push(format!("  {part}"));
            }
        }
    }
    rows
}

fn strip_inline_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' || c == '*' {
            continue;
        }
        if c == '[' {
            let mut label = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == ']' {
                    closed = true;
                    break;
                }
                label.push(next);
            }
            if closed && chars.peek() == Some(&'(') {
                chars.next();
                for next in chars.by_ref() {
                    if next == ')' {
                        break;
                    }
                }
                out.push_str(&label);
            } else {
                out.push('[');
                out.push_str(&label);
                if closed {
                    out.push(']');
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn wrap_note_line(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || width(text) <= max_width {
        return vec![text.to_owned()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        if word.is_empty() {
            continue;
        }
        let needed = if current.is_empty() {
            width(word)
        } else {
            width(&current) + 1 + width(word)
        };
        if needed <= max_width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        } else {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            if width(word) <= max_width {
                current.push_str(word);
            } else {
                let mut chunk = String::new();
                for g in word.graphemes(true) {
                    if width(&chunk) + width(g) > max_width && !chunk.is_empty() {
                        lines.push(std::mem::take(&mut chunk));
                    }
                    chunk.push_str(g);
                }
                current = chunk;
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Build the 2–3 item "What's new for you" digest of changes touching keys or
/// settings the user uses since last launch (`state.updates.since_version`).
pub fn digest_for_user(state: &State) -> Vec<(String, String)> {
    let mut scored: Vec<(u8, usize, ChangeTopic, String, String)> = Vec::new();
    let mut order = 0usize;

    // 1. If a newer release is available with cached tag notes, check its
    //    bullets first for mentions of keys or settings the user uses.
    if let Some(release) = state.update_available()
        && !release.notes.trim().is_empty()
    {
        for bullet in extract_markdown_bullets(&release.notes) {
            if let Some((score, topic, detail)) = match_remote_bullet(state, &bullet) {
                let summary = wrap_note_line(&bullet, NOTE_LINE_MAX_WIDTH)
                    .into_iter()
                    .next()
                    .unwrap_or(bullet);
                scored.push((score + 1, order, topic, summary, detail));
                order += 1;
            }
        }
    }

    // 2. Score bundled release changes since `since_version` (or across recent
    //    shipped releases when no prior version was recorded).
    let since = state.updates.since_version;
    for release in BUNDLED_RELEASES {
        if let Some(since) = since
            && release.version <= since
        {
            continue;
        }
        for entry in release.entries {
            let score = topic_score(state, entry.topic);
            if score > 0 {
                let detail = topic_detail(state, entry.topic, entry.detail);
                scored.push((score, order, entry.topic, entry.summary.to_owned(), detail));
                order += 1;
            }
        }
    }

    // Highest relevance score first, preserving release recency on ties.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut digest = Vec::new();
    let mut seen_topics: Vec<ChangeTopic> = Vec::new();
    let mut seen_summaries: Vec<String> = Vec::new();
    for (_, _, topic, summary, detail) in scored {
        if topic != ChangeTopic::General && seen_topics.contains(&topic) {
            continue;
        }
        if seen_summaries.contains(&summary) {
            continue;
        }
        seen_topics.push(topic);
        seen_summaries.push(summary.clone());
        digest.push((summary, detail));
        if digest.len() >= DIGEST_MAX {
            break;
        }
    }
    digest
}

fn extract_markdown_bullets(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed.starts_with("```")
                || trimmed.starts_with("---")
            {
                return None;
            }
            let body = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("+ "))
                .or_else(|| trimmed.strip_prefix("• "))
                .unwrap_or(trimmed);
            let cleaned = strip_inline_markdown(body)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (!cleaned.is_empty()).then_some(cleaned)
        })
        .collect()
}

fn match_remote_bullet(state: &State, bullet: &str) -> Option<(u8, ChangeTopic, String)> {
    let lower = bullet.to_ascii_lowercase();
    let candidates: &[(ChangeTopic, &[&str], &str)] = &[
        (
            ChangeTopic::Setting(Setting::SidebarWidth),
            &["sidebar_width", "sidebar width", "alt+,", "alt+.", "resize"],
            "sidebar_width",
        ),
        (
            ChangeTopic::Setting(Setting::Previews),
            &["preview", "previews", "preview_time", "preview_activity"],
            "previews",
        ),
        (
            ChangeTopic::Setting(Setting::Avatars),
            &["avatar", "avatars", "profile picture", "refresh-pictures"],
            "avatars",
        ),
        (
            ChangeTopic::Setting(Setting::Transcribe),
            &["transcribe", "whisper", "voice note", "transcription"],
            "voice",
        ),
        (
            ChangeTopic::Setting(Setting::PasteFromSystem),
            &[
                "paste_from_system",
                "clipboard",
                "ctrl+v",
                "ghostty",
                "osc 5522",
            ],
            "ctrl+v",
        ),
        (
            ChangeTopic::Setting(Setting::InlineImages),
            &[
                "inline_images",
                "inline image",
                "kitty",
                "sixel",
                "iterm2",
                "halfblock",
            ],
            "inline_images",
        ),
        (
            ChangeTopic::Setting(Setting::Layout),
            &["bubble", "bubbles", "irc", "layout", "density"],
            "layout",
        ),
        (
            ChangeTopic::Setting(Setting::Allowlist),
            &["allowlist"],
            "allowlist",
        ),
        (
            ChangeTopic::Setting(Setting::UpdateCheck),
            &["updates.check", "update check"],
            "updates.check",
        ),
        (
            ChangeTopic::Action(Action::ViewThread, Focus::Messages),
            &["quote thread", "thread lens", "message.thread"],
            "T, t",
        ),
        (
            ChangeTopic::DisappearingMessages,
            &["disappearing", "expire", "timer"],
            "◷ timer",
        ),
        (
            ChangeTopic::SafetyNumbers,
            &["safety number", "identity key", "verified"],
            "alt+i",
        ),
    ];
    for &(topic, keywords, fallback_detail) in candidates {
        if keywords.iter().any(|kw| lower.contains(kw)) {
            let score = topic_score(state, topic);
            if score > 0 {
                let detail = topic_detail(state, topic, fallback_detail);
                return Some((score, topic, detail));
            }
        }
    }
    None
}

/// How strongly `topic` touches the user's keys or settings:
/// - `3`: explicitly customized setting (`changed(state)`), rebound key in
///   `config.keys`, or recently run command in `recent_commands`.
/// - `2`: active feature in the user's current config/conversations.
/// - `1`: bound default key in the user's active keymap.
/// - `0`: not used by this setup.
fn topic_score(state: &State, topic: ChangeTopic) -> u8 {
    match topic {
        ChangeTopic::Setting(setting) => {
            if setting_group_changed(state, setting) {
                3
            } else if setting_active(state, setting) {
                2
            } else {
                0
            }
        }
        ChangeTopic::Action(action, focus) => {
            let rebound =
                state.keymap.keys_for(action, focus) != Keymap::default().keys_for(action, focus);
            let recent = state
                .recent_commands
                .contains(&PaletteCommand::Action(action));
            if rebound || recent {
                3
            } else {
                u8::from(state.keymap.hint(&[action], focus).is_some())
            }
        }
        ChangeTopic::Nicknames => {
            if state.nicknames.is_empty() {
                0
            } else {
                3
            }
        }
        ChangeTopic::DisappearingMessages => {
            if state
                .conversations
                .iter()
                .any(|c| c.disappearing_timer.is_some())
            {
                3
            } else {
                1
            }
        }
        ChangeTopic::SafetyNumbers => {
            if state
                .identities
                .values()
                .any(|i| i.verified != VerificationState::Default || i.key_changed)
            {
                3
            } else {
                1
            }
        }
        ChangeTopic::General => 0,
    }
}

fn setting_group_changed(state: &State, setting: Setting) -> bool {
    if setting.changed(state) {
        return true;
    }
    match setting {
        Setting::Previews => {
            Setting::PreviewTime.changed(state) || Setting::PreviewActivity.changed(state)
        }
        Setting::Layout => {
            Setting::BubbleStyle.changed(state)
                || Setting::BubblePadding.changed(state)
                || Setting::MatchWidths.changed(state)
                || Setting::Density.changed(state)
        }
        Setting::Transcribe => {
            Setting::TranscribeAuto.changed(state)
                || Setting::TranscribeMaxSecs.changed(state)
                || Setting::TranscribeModel.changed(state)
                || Setting::TranscribeLanguage.changed(state)
                || Setting::TranscribeCommand.changed(state)
                || Setting::TranscribeEndpoint.changed(state)
                || Setting::TranscribeRemoteModel.changed(state)
        }
        Setting::Allowlist => state.list.allowlist,
        Setting::Colors
        | Setting::Emoji
        | Setting::SidebarWidth
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
        | Setting::PreviewTime
        | Setting::PreviewActivity
        | Setting::Avatars
        | Setting::InlineImages
        | Setting::AutoDownload
        | Setting::TranscribeAuto
        | Setting::TranscribeMaxSecs
        | Setting::TranscribeModel
        | Setting::TranscribeLanguage
        | Setting::TranscribeCommand
        | Setting::TranscribeEndpoint
        | Setting::TranscribeRemoteModel
        | Setting::Mouse
        | Setting::PasteFromSystem
        | Setting::Beam
        | Setting::RestoreSession
        | Setting::AutoLock
        | Setting::UpdateCheck => false,
    }
}

fn setting_active(state: &State, setting: Setting) -> bool {
    match setting {
        Setting::Previews | Setting::PreviewTime | Setting::PreviewActivity => {
            state.config.conversations.previews
        }
        Setting::Avatars => state.config.conversations.avatars != Avatars::Off,
        Setting::SidebarWidth => state.config.ui.sidebar_width != 30,
        Setting::PasteFromSystem => state.config.media.paste_from_system,
        Setting::InlineImages => state.config.media.inline_images != InlineImages::Off,
        Setting::Transcribe | Setting::TranscribeAuto => {
            state.config.voice.transcribe != TranscribeMode::Off
        }
        Setting::UpdateCheck => state.config.updates.check,
        Setting::Allowlist => state.list.allowlist || state.config.conversations.allowlist,
        Setting::RestoreSession => state.config.conversations.restore_session,
        Setting::AutoLock => state.config.security.auto_lock_mins > 0,
        Setting::Layout => true,
        Setting::Colors
        | Setting::Emoji
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
        | Setting::AutoDownload
        | Setting::TranscribeMaxSecs
        | Setting::TranscribeModel
        | Setting::TranscribeLanguage
        | Setting::TranscribeCommand
        | Setting::TranscribeEndpoint
        | Setting::TranscribeRemoteModel
        | Setting::Mouse => false,
    }
}

fn topic_detail(state: &State, topic: ChangeTopic, fallback: &str) -> String {
    match topic {
        ChangeTopic::Action(action, focus) => state
            .keymap
            .hint(&[action], focus)
            .unwrap_or_else(|| fallback.to_owned()),
        ChangeTopic::Setting(Setting::SidebarWidth) => state
            .keymap
            .hint(&[Action::NarrowSidebar, Action::WidenSidebar], Focus::Input)
            .unwrap_or_else(|| format!("width {}", state.config.ui.sidebar_width)),
        ChangeTopic::Setting(setting) => {
            let val = value_title(state, setting);
            let candidate = format!("{fallback}: {val}");
            if width(&candidate) <= 18 {
                candidate
            } else {
                fallback.to_owned()
            }
        }
        ChangeTopic::Nicknames
        | ChangeTopic::DisappearingMessages
        | ChangeTopic::SafetyNumbers
        | ChangeTopic::General => fallback.to_owned(),
    }
}

impl crate::app::state::State {
    /// Why update checks can't run here, if they can't.
    pub fn update_checks_unavailable(&self) -> Option<Unavailable> {
        if self.is_demo() {
            Some(Unavailable::Demo)
        } else if !self.updates.built {
            Some(Unavailable::NotBuilt)
        } else if Version::current().is_none() {
            Some(Unavailable::Development)
        } else {
            None
        }
    }

    /// Update checks are on and can run.
    pub fn update_checks_active(&self) -> bool {
        self.config.updates.check && self.update_checks_unavailable().is_none()
    }

    /// The release to tell the user about (the footer marker, the logo
    /// screen, About and the palette), while update checks are on.
    pub fn update_available(&self) -> Option<&Release> {
        self.updates
            .available
            .as_ref()
            .filter(|_| self.update_checks_active())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn versions_parse_with_or_without_a_v() {
        assert_eq!(Version::parse("0.2.0"), Some(v(0, 2, 0)));
        assert_eq!(Version::parse("v1.10.3"), Some(v(1, 10, 3)));
        assert_eq!(Version::parse(" v0.1.3\n"), Some(v(0, 1, 3)));
    }

    #[test]
    fn pre_releases_and_junk_are_not_versions() {
        for text in [
            "0.2.0-dev",
            "0.2.0-rc.1",
            "0.2.0+build",
            "0.2",
            "0.2.0.1",
            "v",
            "",
            "x.y.z",
            "0..1",
            "+1.2.3",
            "1.2.-3",
        ] {
            assert_eq!(Version::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn versions_compare_by_number_not_text() {
        assert!(v(0, 10, 0) > v(0, 9, 9));
        assert!(v(1, 0, 0) > v(0, 99, 99));
        assert!(v(0, 1, 4) > v(0, 1, 3));
    }

    #[test]
    fn the_current_version_is_this_crate_s() {
        let current = Version::current().map(|c| c.to_string());
        let expected = env!("CARGO_PKG_VERSION");
        if expected.contains('-') {
            assert_eq!(current, None);
        } else {
            assert_eq!(current.as_deref(), Some(expected));
        }
    }

    #[test]
    fn a_release_round_trips_through_app_state() {
        let release = Release::new(v(0, 2, 0), 1_760_000_000);
        assert_eq!(release.to_stored(), "0.2.0 1760000000");
        assert_eq!(Release::from_stored(&release.to_stored()), Some(release));

        let with_notes = Release::with_notes(
            v(0, 2, 0),
            1_760_000_000,
            "- Added offline release notes\n- Added digest",
        );
        assert_eq!(
            with_notes.to_stored(),
            "0.2.0 1760000000\n- Added offline release notes\n- Added digest"
        );
        assert_eq!(
            Release::from_stored(&with_notes.to_stored()),
            Some(with_notes)
        );
        assert_eq!(Release::from_stored("0.2.0"), None);
        assert_eq!(Release::from_stored("0.2.0-dev 1"), None);
        assert_eq!(Release::from_stored("0.2.0 soon"), None);
    }

    #[test]
    fn a_release_is_announced_once_newer_and_past_the_grace_period() {
        let release = Release::new(v(0, 2, 0), 1_000_000);
        let later = 1_000_000 + GRACE_SECS;
        assert!(
            !release.announceable(v(0, 1, 3), later - 1),
            "still in grace"
        );
        assert!(release.announceable(v(0, 1, 3), later));
        assert!(!release.announceable(v(0, 2, 0), later), "same version");
        assert!(!release.announceable(v(0, 3, 0), later), "older than ours");
    }

    #[test]
    fn release_notes_link_to_the_tag() {
        let release = Release::new(v(0, 2, 0), 0);
        assert_eq!(release.url(), format!("{REPOSITORY}/releases/tag/v0.2.0"));
        assert!(
            REPOSITORY.starts_with("https://github.com/"),
            "{REPOSITORY}"
        );
    }

    #[test]
    fn markdown_release_notes_strip_syntax_and_wrap_for_80x24() {
        let raw = "## What's Changed\n\
                   - Added `[ui] sidebar_width` resize with **mouse drag** and `Alt+,` / `Alt+.` keys so wide lists fit comfortably\n\
                   - Fixed [issue #1](https://github.com/harababurel/faro/issues/1) in Ghostty\n";
        let rows = parse_markdown_notes(raw);
        assert_ne!(rows, [] as [String; 0]);
        for row in &rows {
            assert!(
                width(row) <= NOTE_LINE_MAX_WIDTH + 2,
                "row too wide for 80x24 palette: {row:?} ({})",
                width(row)
            );
            assert!(!row.contains("https://"));
            assert!(!row.contains('`'));
        }
        assert!(rows[0].starts_with("Added [ui] sidebar_width"));
        assert!(rows.iter().any(|r| r.contains("Fixed issue #1 in Ghostty")));
    }
}
