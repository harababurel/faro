//! Messages and what hangs off them.

use super::{ConversationId, MessageBody, MessageRef, ServiceId, Timestamp};

/// Signal's MIME type for the full UTF-8 body attachment of a message longer
/// than [`LONG_TEXT_THRESHOLD`] bytes.
pub const LONG_TEXT_MIME: &str = "text/x-signal-plain";

/// Maximum inline `DataMessage.body` length in UTF-8 bytes before Signal
/// clients truncate the inline preview and expect a [`LONG_TEXT_MIME`]
/// attachment carrying the full body.
pub const LONG_TEXT_THRESHOLD: usize = 2048;

/// Upper bound (1 MiB) on [`LONG_TEXT_MIME`] attachments that faro will
/// automatically inline into `Message::body`.
pub const LONG_TEXT_MAX_BYTES: u64 = 1024 * 1024;

/// Truncate `text` to at most [`LONG_TEXT_THRESHOLD`] UTF-8 bytes without
/// splitting a UTF-8 scalar or common multi-codepoint emoji sequence.
#[must_use]
pub fn truncate_long_text(text: &str) -> &str {
    if text.len() <= LONG_TEXT_THRESHOLD {
        return text;
    }
    let mut end = text.floor_char_boundary(LONG_TEXT_THRESHOLD);
    while end > 0 {
        let next = text.get(end..).and_then(|s| s.chars().next());
        let prev = text.get(..end).and_then(|s| s.chars().next_back());
        let joins_next = next.is_some_and(|c| {
            c == '\u{200D}'
                || c == '\u{FE0E}'
                || c == '\u{FE0F}'
                || ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
                || ('\u{0300}'..='\u{036F}').contains(&c)
        });
        let after_zwj = prev == Some('\u{200D}');
        let splits_flag = prev.is_some_and(|p| ('\u{1F1E6}'..='\u{1F1FF}').contains(&p))
            && next.is_some_and(|n| ('\u{1F1E6}'..='\u{1F1FF}').contains(&n))
            && text.get(..end).map_or(0, |s| {
                s.chars()
                    .rev()
                    .take_while(|&c| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c))
                    .count()
            }) % 2
                == 1;
        if !joins_next && !after_zwj && !splits_flag {
            break;
        }
        end = prev.map_or(0, |p| end - p.len_utf8());
    }
    text.get(..end).unwrap_or("")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub conversation: ConversationId,
    pub sender: ServiceId,
    /// The sender's timestamp; with `sender`, this is the message's Signal identity.
    pub sent_at: Timestamp,
    pub direction: Direction,
    /// `None` for attachment-only messages and after deletion.
    pub body: Option<MessageBody>,
    /// People mentioned in `body` (D72); draw them with [`mention_parts`].
    pub mentions: Vec<Mention>,
    pub quote: Option<Quote>,
    pub previews: Vec<LinkPreview>,
    pub attachments: Vec<AttachmentRef>,
    /// At most one per sender (Signal replaces a person's previous reaction).
    pub reactions: Vec<Reaction>,
    pub edited_at: Option<Timestamp>,
    /// Deleted for everyone; shown as a placeholder.
    pub deleted: bool,
    /// Disappearing-message timer duration in seconds, if this message expires (D165).
    pub expires_in_secs: Option<u32>,
    /// When the expiration countdown started: `sent_at` for outgoing messages,
    /// or when read (`mark_read` / `ReadElsewhere`) for incoming messages (D165).
    pub expire_started_at: Option<Timestamp>,
    /// When `Some`, this row is an inline system event (group change, timer
    /// change, or call) rather than a regular chat message (D167).
    pub system_event: Option<SystemEvent>,
}

/// An inline system event in a conversation's timeline (D167).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemEvent {
    /// Disappearing-message timer changed (`None` = turned off).
    DisappearingTimer { seconds: Option<u32> },
    /// Group renamed (or name cleared when `title` is empty).
    GroupRenamed { title: String },
    /// Members joined or were added to the group by `message.sender`.
    MembersJoined { members: Vec<ServiceId> },
    /// Members left or were removed from the group by `message.sender`.
    MembersLeft { members: Vec<ServiceId> },
    /// A voice, video, or group call event.
    Call { kind: CallKind, state: CallState },
}

/// Kind of Signal call (D167).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    Voice,
    Video,
    Group,
}

/// Lifecycle state of a Signal call event (D167).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// Actively ringing / offer received.
    Incoming,
    /// Unanswered, declined, or missed.
    Missed,
    /// Answered on this account's primary/other device, or completed outgoing call.
    Accepted,
}

impl Message {
    /// A system status line in `conversation` (group change, timer change, or call; D167).
    #[must_use]
    pub fn system(
        conversation: ConversationId,
        sender: ServiceId,
        sent_at: Timestamp,
        direction: Direction,
        event: SystemEvent,
    ) -> Self {
        Self {
            conversation,
            sender,
            sent_at,
            direction,
            body: None,
            mentions: Vec::new(),
            quote: None,
            previews: Vec::new(),
            attachments: Vec::new(),
            reactions: Vec::new(),
            edited_at: None,
            deleted: false,
            expires_in_secs: None,
            expire_started_at: None,
            system_event: Some(event),
        }
    }

    /// Whether this row is an inline system event rather than a user message (D167).
    #[must_use]
    pub fn is_system(&self) -> bool {
        self.system_event.is_some()
    }

    pub fn reference(&self) -> MessageRef {
        MessageRef {
            sender: self.sender,
            sent_at: self.sent_at,
        }
    }

    /// Set or (with `None`) remove `sender`'s reaction.
    pub fn apply_reaction(&mut self, sender: ServiceId, emoji: Option<String>) {
        self.reactions.retain(|r| r.sender != sender);
        if let Some(emoji) = emoji {
            self.reactions.push(Reaction { sender, emoji });
        }
    }

    /// Whether the message mentions `who`.
    pub fn mentions_person(&self, who: ServiceId) -> bool {
        !self.deleted && self.mentions.iter().any(|m| m.who == who)
    }

    /// Whether the message's body contains any [`TextStyle::Spoiler`] span (D177).
    pub fn has_spoilers(&self) -> bool {
        !self.deleted
            && self.body.as_ref().is_some_and(|b| {
                b.styles()
                    .iter()
                    .any(|s| s.style == TextStyle::Spoiler && s.len > 0)
            })
    }

    /// When this message will expire and be deleted, if its countdown has started (D165).
    pub fn expires_at(&self) -> Option<Timestamp> {
        let secs = self.expires_in_secs.filter(|&s| s > 0)?;
        let start = self.expire_started_at.or(match self.direction {
            Direction::Outgoing(_) => Some(self.sent_at),
            Direction::Incoming => None,
        })?;
        Some(Timestamp::from_millis(
            start.as_millis().saturating_add(u64::from(secs) * 1000),
        ))
    }
}

/// `who` is mentioned in place of `body[start..start + len]` (byte offsets;
/// Signal puts U+FFFC there and the name is drawn instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mention {
    pub who: ServiceId,
    pub start: usize,
    pub len: usize,
}

/// Signal's five inline text styles (`BodyRange.style`, D177).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextStyle {
    Bold,
    Italic,
    Spoiler,
    Strikethrough,
    Monospace,
}

impl TextStyle {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Spoiler => "spoiler",
            Self::Strikethrough => "strikethrough",
            Self::Monospace => "monospace",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bold" => Some(Self::Bold),
            "italic" => Some(Self::Italic),
            "spoiler" => Some(Self::Spoiler),
            "strikethrough" => Some(Self::Strikethrough),
            "monospace" => Some(Self::Monospace),
            _ => None,
        }
    }
}

/// An inline [`TextStyle`] range in a [`MessageBody`], by UTF-8 byte offset
/// and byte length (D177).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StyleSpan {
    pub start: usize,
    pub len: usize,
    pub style: TextStyle,
}

impl StyleSpan {
    pub const fn end(self) -> usize {
        self.start.saturating_add(self.len)
    }
}

/// Active [`TextStyle`]s on a segment of message text (D177).
#[expect(
    clippy::struct_excessive_bools,
    reason = "StyleFlags represents 5 orthogonal inline text styles"
)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StyleFlags {
    pub bold: bool,
    pub italic: bool,
    pub spoiler: bool,
    pub strikethrough: bool,
    pub monospace: bool,
}

impl StyleFlags {
    pub const fn is_empty(self) -> bool {
        !self.bold && !self.italic && !self.spoiler && !self.strikethrough && !self.monospace
    }

    pub fn add(&mut self, style: TextStyle) {
        match style {
            TextStyle::Bold => self.bold = true,
            TextStyle::Italic => self.italic = true,
            TextStyle::Spoiler => self.spoiler = true,
            TextStyle::Strikethrough => self.strikethrough = true,
            TextStyle::Monospace => self.monospace = true,
        }
    }
}

/// Normalize style spans against `text`: drop empty or out-of-bounds spans or
/// spans not on UTF-8 char boundaries, sort by `(style, start)`, merge
/// overlapping or adjacent spans of the same style, and sort by `(start, len, style)`.
pub fn normalize_styles(text: &str, spans: Vec<StyleSpan>) -> Vec<StyleSpan> {
    let mut valid: Vec<StyleSpan> = spans
        .into_iter()
        .filter(|s| {
            let Some(end) = s.start.checked_add(s.len) else {
                return false;
            };
            s.len > 0
                && end <= text.len()
                && text.is_char_boundary(s.start)
                && text.is_char_boundary(end)
        })
        .collect();
    if valid.is_empty() {
        return valid;
    }
    valid.sort_by_key(|s| (s.style, s.start, s.len));
    let mut merged: Vec<StyleSpan> = Vec::with_capacity(valid.len());
    for span in valid {
        if let Some(last) = merged.last_mut()
            && last.style == span.style
            && span.start <= last.end()
        {
            let new_end = last.end().max(span.end());
            last.len = new_end - last.start;
        } else {
            merged.push(span);
        }
    }
    merged.sort_by_key(|s| (s.start, s.len, s.style));
    merged
}

/// A piece of a message body: text as written, or a mention to draw as a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyPart<'a> {
    Text(&'a str),
    Mention(ServiceId),
}

/// One piece of a message body after splitting out valid [`Mention`]s and
/// [`StyleSpan`]s (D177).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyledSegment<'a> {
    Text(&'a str, StyleFlags),
    Mention(ServiceId, StyleFlags),
}

/// `body` cut into text and mentions, in order. Mentions that don't fit the
/// body (out of range, overlapping an earlier one, or not on character
/// boundaries) are left out, so whatever a client sends can't break drawing.
pub fn mention_parts<'a>(body: &'a str, mentions: &[Mention]) -> Vec<BodyPart<'a>> {
    let mut sorted: Vec<&Mention> = mentions.iter().collect();
    sorted.sort_by_key(|m| (m.start, m.len));
    let mut parts = Vec::new();
    let mut at = 0;
    for mention in sorted {
        let Some(end) = mention.start.checked_add(mention.len) else {
            continue;
        };
        if mention.start < at || mention.len == 0 {
            continue;
        }
        let (Some(before), Some(_)) = (body.get(at..mention.start), body.get(mention.start..end))
        else {
            continue;
        };
        if !before.is_empty() {
            parts.push(BodyPart::Text(before));
        }
        parts.push(BodyPart::Mention(mention.who));
        at = end;
    }
    if let Some(rest) = body.get(at..).filter(|rest| !rest.is_empty()) {
        parts.push(BodyPart::Text(rest));
    }
    parts
}

/// Split `body` into [`StyledSegment`]s carrying their active [`StyleFlags`]
/// and [`Mention`]s (D177).
pub fn styled_parts<'a>(body: &'a MessageBody, mentions: &[Mention]) -> Vec<StyledSegment<'a>> {
    let text = body.expose();
    let styles = body.styles();
    if mentions.is_empty() && styles.is_empty() {
        if text.is_empty() {
            return Vec::new();
        }
        return vec![StyledSegment::Text(text, StyleFlags::default())];
    }
    let mut sorted_mentions: Vec<&Mention> = mentions.iter().collect();
    sorted_mentions.sort_by_key(|m| (m.start, m.len));
    let mut valid_mentions = Vec::with_capacity(sorted_mentions.len());
    let mut m_end = 0;
    for mention in sorted_mentions {
        let Some(end) = mention.start.checked_add(mention.len) else {
            continue;
        };
        if mention.start < m_end || mention.len == 0 {
            continue;
        }
        if text.get(mention.start..end).is_some() {
            m_end = end;
            valid_mentions.push(*mention);
        }
    }

    let flags_for = |start: usize, end: usize| -> StyleFlags {
        let mut flags = StyleFlags::default();
        for s in styles {
            if s.start < end && s.end() > start {
                flags.add(s.style);
            }
        }
        flags
    };

    let mut cuts = vec![0, text.len()];
    for m in &valid_mentions {
        cuts.push(m.start);
        cuts.push(m.start + m.len);
    }
    for s in styles {
        if text.is_char_boundary(s.start) && text.is_char_boundary(s.end()) {
            cuts.push(s.start);
            cuts.push(s.end());
        }
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut out = Vec::new();
    let mut at = 0;
    let mut m_idx = 0;
    while at < text.len() {
        if let Some(m) = valid_mentions.get(m_idx) {
            if m.start == at {
                let end = m.start + m.len;
                out.push(StyledSegment::Mention(m.who, flags_for(m.start, end)));
                at = end;
                m_idx += 1;
                continue;
            } else if m.start < at {
                m_idx += 1;
                continue;
            }
        }
        let next_cut = cuts.iter().copied().find(|&c| c > at).unwrap_or(text.len());
        let next_end = match valid_mentions.get(m_idx) {
            Some(m) if m.start > at => next_cut.min(m.start),
            _ => next_cut,
        };
        if let Some(slice) = text.get(at..next_end)
            && !slice.is_empty()
        {
            out.push(StyledSegment::Text(slice, flags_for(at, next_end)));
        }
        at = next_end;
    }
    out
}

/// Mask any [`TextStyle::Spoiler`] spans in `body` with `'░'` characters so
/// previews and notifications never leak hidden spoiler text (D177).
pub fn mask_spoilers(body: &MessageBody) -> String {
    let spoilers: Vec<StyleSpan> = body
        .styles()
        .iter()
        .copied()
        .filter(|s| s.style == TextStyle::Spoiler)
        .collect();
    let text = body.expose();
    if spoilers.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for span in spoilers {
        let end = span.end().min(text.len());
        if span.start < at || !text.is_char_boundary(span.start) || !text.is_char_boundary(end) {
            continue;
        }
        if let Some(before) = text.get(at..span.start) {
            out.push_str(before);
        }
        if let Some(secret) = text.get(span.start..end) {
            for ch in secret.chars() {
                if ch == '\n' {
                    out.push('\n');
                } else {
                    out.push('░');
                }
            }
        }
        at = end;
    }
    if let Some(rest) = text.get(at..) {
        out.push_str(rest);
    }
    out
}

/// Who wrote the message, from this account's point of view. Messages sent from
/// the owner's phone (sync messages) are `Outgoing` too (DESIGN.md §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Incoming,
    Outgoing(DeliveryStatus),
}

/// Delivery progress of an outgoing message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Sending,
    Sent,
    Delivered,
    Read,
    Failed,
}

impl DeliveryStatus {
    /// Combine the current status with an update. Receipts can arrive late, twice or
    /// out of order, so a confirmed status never moves backwards. `Failed` replaces
    /// only `Sending`; a retry (`Sending`) or any confirmation replaces `Failed`.
    #[must_use]
    pub fn update(self, new: Self) -> Self {
        match (self, new) {
            (Self::Sending | Self::Failed, _) => new,
            (_, Self::Sending | Self::Failed) => self,
            // Spelled out (no `_`) so a new variant fails to compile here.
            (
                Self::Sent | Self::Delivered | Self::Read,
                Self::Sent | Self::Delivered | Self::Read,
            ) => {
                if new.confirmation_rank() > self.confirmation_rank() {
                    new
                } else {
                    self
                }
            }
        }
    }

    fn confirmation_rank(self) -> u8 {
        match self {
            Self::Sending | Self::Failed => 0,
            Self::Sent => 1,
            Self::Delivered => 2,
            Self::Read => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub sender: ServiceId,
    pub emoji: String,
}

/// A reply's reference to an earlier message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    pub target: MessageRef,
    /// The quoted text as the sender saw it; the original may be missing locally.
    pub preview: Option<MessageBody>,
}

/// Link preview metadata attached by the sender (`DataMessage.preview`; #02).
/// Remote thumbnail pointers (`Preview.image`) are ignored so viewing a chat
/// never fetches remote preview images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
}

impl LinkPreview {
    /// Host/site name extracted from `url` (without scheme, `www.`, port, or path).
    #[must_use]
    pub fn site(&self) -> Option<&str> {
        let rest = self
            .url
            .trim()
            .split_once("://")
            .map_or(self.url.trim(), |(_, after)| after);
        let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = host_port
            .rsplit_once('@')
            .map_or(host_port, |(_, h)| h)
            .split(':')
            .next()
            .unwrap_or("")
            .trim_matches('.');
        let site = host
            .strip_prefix("www.")
            .or_else(|| host.strip_prefix("WWW."))
            .unwrap_or(host);
        (!site.is_empty()).then_some(site)
    }
}

/// Backend-specific handle used to fetch an attachment's content. Opaque to
/// faro; the presage backend keeps the attachment's key in it (D75), so
/// `Debug` shows only its start.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct AttachmentId(pub String);

impl std::fmt::Debug for AttachmentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let start: String = self.0.chars().take(8).collect();
        if start.len() < self.0.len() {
            write!(f, "AttachmentId({start}…)")
        } else {
            write!(f, "AttachmentId({start})")
        }
    }
}

/// Where a voice note transcript came from (D82).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptSource {
    /// Transcribed on this machine (`[voice] transcribe = "local"`).
    Local,
    /// Transcribed by a remote endpoint (`[voice] transcribe = "remote"`).
    Remote,
    /// A canned transcript of one of the web demo's voice notes (D117).
    Demo,
}

impl TranscriptSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Demo => "demo",
        }
    }
}

/// A transcript as one paragraph: Whisper writes each timing segment on a
/// line of its own (often mid-sentence), so every run of whitespace becomes
/// one space (D145).
pub fn transcript_paragraph(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Metadata about an attachment; the content is fetched on demand (D75, D80–D82).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef {
    pub id: AttachmentId,
    pub content_type: String,
    pub file_name: Option<String>,
    pub size: Option<u64>,
    /// Pixel size of images, when the sender gave it or the file was read.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub status: AttachmentStatus,
    /// Whether Signal flagged the attachment as a voice note (`flags & 1 != 0`; D80).
    pub voice_note: bool,
    /// Duration in seconds for voice notes / audio clips when known (D80).
    pub duration_secs: Option<u32>,
    /// 16-bar RMS amplitude envelope (`0..=7` per bar; D80).
    pub waveform: Option<Vec<u8>>,
    /// Persisted speech-to-text transcript for a voice note (D82).
    pub transcript: Option<String>,
    /// What produced `transcript` (D82, D117).
    pub transcript_source: Option<TranscriptSource>,
}

impl AttachmentRef {
    /// An attachment as a backend reports it: not downloaded, size unknown.
    pub fn remote(id: impl Into<String>, content_type: impl Into<String>) -> Self {
        Self {
            id: AttachmentId(id.into()),
            content_type: content_type.into(),
            file_name: None,
            size: None,
            width: None,
            height: None,
            status: AttachmentStatus::Remote,
            voice_note: false,
            duration_secs: None,
            waveform: None,
            transcript: None,
            transcript_source: None,
        }
    }

    /// Whether it's a picture faro can draw (D78).
    pub fn is_image(&self) -> bool {
        matches!(
            self.content_type.to_ascii_lowercase().as_str(),
            "image/jpeg" | "image/jpg" | "image/png" | "image/gif" | "image/webp"
        )
    }

    /// A Signal sticker (D79): a WebP image without a file name.
    pub fn is_sticker(&self) -> bool {
        self.file_name.is_none() && self.content_type.eq_ignore_ascii_case("image/webp")
    }

    /// Whether this attachment is a video clip (`video/*`).
    pub fn is_video(&self) -> bool {
        self.content_type.to_ascii_lowercase().starts_with("video/")
    }

    /// Whether this attachment belongs in the visual media gallery (images,
    /// stickers, and videos; #21).
    pub fn is_visual_media(&self) -> bool {
        self.is_image() || self.is_video()
    }

    /// Whether this attachment is a voice note (D80): explicitly flagged by
    /// Signal (`voice_note`), or an `audio/*` clip with no file name or a
    /// `voice-*` / `signal-voice*` file name.
    pub fn is_voice_note(&self) -> bool {
        self.voice_note
            || (self.content_type.to_ascii_lowercase().starts_with("audio/")
                && self
                    .file_name
                    .as_deref()
                    .is_none_or(|n| n.starts_with("voice-") || n.starts_with("signal-voice")))
    }

    /// Whether this attachment is Signal's [`LONG_TEXT_MIME`] payload carrying
    /// the full body of a message longer than [`LONG_TEXT_THRESHOLD`] bytes.
    #[must_use]
    pub fn is_long_text(&self) -> bool {
        self.content_type.eq_ignore_ascii_case(LONG_TEXT_MIME)
    }

    /// The downloaded copy, if there is one.
    pub fn local_path(&self) -> Option<&std::path::Path> {
        match &self.status {
            AttachmentStatus::Downloaded(path) => Some(path),
            AttachmentStatus::Remote | AttachmentStatus::Failed => None,
        }
    }

    /// Whether the attachment has been downloaded to disk.
    pub fn is_downloaded(&self) -> bool {
        self.local_path().is_some()
    }
}

/// Whether faro has an attachment's content (D75). "Downloading" is only
/// known to the running app.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AttachmentStatus {
    #[default]
    Remote,
    Downloaded(std::path::PathBuf),
    /// The last download failed; the user can try again.
    Failed,
}

/// Format a byte count for display (`512 B`, `12.1 KB`, `5.0 MB`).
#[expect(
    clippy::cast_precision_loss,
    reason = "sizes are shown to one decimal; precision beyond 2^52 bytes doesn't matter"
)]
pub fn format_byte_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS.get(unit).copied().unwrap_or("TB"))
}

/// Format a disappearing-message timer duration compactly (`30s`, `5m`, `1h`,
/// `1d`, `1w`, `4w`; D165).
pub fn format_disappearing_timer(secs: u32) -> String {
    const MINUTE: u32 = 60;
    const HOUR: u32 = 3_600;
    const DAY: u32 = 86_400;
    const WEEK: u32 = 604_800;
    if secs == 0 {
        return "off".to_owned();
    }
    if secs.is_multiple_of(WEEK) {
        format!("{}w", secs / WEEK)
    } else if secs.is_multiple_of(DAY) {
        format!("{}d", secs / DAY)
    } else if secs.is_multiple_of(HOUR) {
        format!("{}h", secs / HOUR)
    } else if secs.is_multiple_of(MINUTE) {
        format!("{}m", secs / MINUTE)
    } else if secs >= DAY {
        format!("{}d", secs / DAY)
    } else if secs >= HOUR {
        format!("{}h", secs / HOUR)
    } else if secs >= MINUTE {
        format!("{}m", secs / MINUTE)
    } else {
        format!("{secs}s")
    }
}

/// Format the remaining lifetime until `expires_at` at `now` (`4m`, `23h`, `6d`; D165).
pub fn format_disappearing_remaining(expires_at: Timestamp, now: Timestamp) -> String {
    let remaining_ms = expires_at.as_millis().saturating_sub(now.as_millis());
    let secs = u32::try_from(remaining_ms.div_ceil(1000).max(1)).unwrap_or(u32::MAX);
    format_disappearing_timer(secs)
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::DeliveryStatus::{Delivered, Failed, Read, Sending, Sent};
    use super::*;

    #[test]
    fn status_never_regresses() {
        assert_eq!(Sending.update(Sent), Sent);
        assert_eq!(Sent.update(Read), Read);
        assert_eq!(Read.update(Delivered), Read); // late delivery receipt
        assert_eq!(Delivered.update(Delivered), Delivered);
        assert_eq!(Delivered.update(Sending), Delivered);
    }

    #[test]
    fn failure_only_replaces_sending() {
        assert_eq!(Sending.update(Failed), Failed);
        assert_eq!(Sent.update(Failed), Sent);
        assert_eq!(Failed.update(Sending), Sending); // retry
        assert_eq!(Failed.update(Delivered), Delivered);
    }

    #[test]
    fn one_reaction_per_sender() {
        let alice = ServiceId::Aci(Uuid::from_u128(1));
        let bob = ServiceId::Aci(Uuid::from_u128(2));
        let mut msg = Message {
            conversation: ConversationId::Direct(alice),
            sender: alice,
            sent_at: Timestamp::from_millis(1),
            direction: Direction::Incoming,
            body: Some(MessageBody::new("hi")),
            mentions: vec![],
            quote: None,
            previews: vec![],
            attachments: vec![],
            reactions: vec![],
            edited_at: None,
            deleted: false,
            expires_in_secs: None,
            expire_started_at: None,
            system_event: None,
        };
        msg.apply_reaction(bob, Some("👍".into()));
        msg.apply_reaction(alice, Some("❤️".into()));
        msg.apply_reaction(bob, Some("😂".into()));
        assert_eq!(msg.reactions.len(), 2);
        assert!(msg.reactions.contains(&Reaction {
            sender: bob,
            emoji: "😂".into()
        }));
        msg.apply_reaction(bob, None);
        assert_eq!(msg.reactions.len(), 1);
    }

    #[test]
    fn mentions_split_the_body() {
        let alice = ServiceId::Aci(Uuid::from_u128(1));
        let bob = ServiceId::Aci(Uuid::from_u128(2));
        let body = "hi \u{FFFC} and \u{FFFC}!";
        let at = |who, start| Mention { who, start, len: 3 };
        // Given out of order; sorted by position.
        let parts = mention_parts(body, &[at(bob, 11), at(alice, 3)]);
        assert_eq!(
            parts,
            vec![
                BodyPart::Text("hi "),
                BodyPart::Mention(alice),
                BodyPart::Text(" and "),
                BodyPart::Mention(bob),
                BodyPart::Text("!"),
            ]
        );
        assert_eq!(
            mention_parts("\u{FFFC}", &[at(alice, 0)]),
            vec![BodyPart::Mention(alice)]
        );
    }

    #[test]
    fn mentions_that_dont_fit_are_left_out() {
        let alice = ServiceId::Aci(Uuid::from_u128(1));
        let body = "hé \u{FFFC}";
        let parts = |mentions: &[Mention]| mention_parts(body, mentions);
        let whole = vec![BodyPart::Text(body)];
        let at = |start, len| Mention {
            who: alice,
            start,
            len,
        };
        assert_eq!(parts(&[at(2, 1)]), whole, "inside a character");
        assert_eq!(parts(&[at(4, 9)]), whole, "past the end");
        assert_eq!(parts(&[at(usize::MAX, 2)]), whole, "overflowing");
        assert_eq!(parts(&[at(0, 0)]), whole, "empty");
        assert_eq!(
            parts(&[at(4, 3), at(5, 2)]),
            vec![BodyPart::Text("hé "), BodyPart::Mention(alice)],
            "overlapping"
        );
    }

    #[test]
    fn attachment_id_redacts_keys_and_helpers_classify_media() {
        let short = AttachmentId("att-1".into());
        assert_eq!(format!("{short:?}"), "AttachmentId(att-1)");
        let long = AttachmentId("p1:0123456789abcdef_secret_key".into());
        assert_eq!(format!("{long:?}"), "AttachmentId(p1:01234…)");

        let mut photo = AttachmentRef::remote("a", "image/JPEG");
        assert!(photo.is_image());
        assert!(!photo.is_sticker());
        assert_eq!(photo.local_path(), None);
        photo.status = AttachmentStatus::Downloaded("/tmp/a.jpg".into());
        assert_eq!(photo.local_path(), Some(std::path::Path::new("/tmp/a.jpg")));

        let sticker = AttachmentRef::remote("s", "image/webp");
        assert!(sticker.is_image() && sticker.is_sticker());
        let doc = AttachmentRef::remote("d", "application/pdf");
        assert!(!doc.is_image());
        assert!(!doc.is_long_text());
        let long_txt = AttachmentRef::remote("l", "text/x-signal-plain");
        assert!(long_txt.is_long_text());
    }

    #[test]
    fn truncate_long_text_keeps_graphemes_intact_at_boundary() {
        let short = "a".repeat(LONG_TEXT_THRESHOLD);
        assert_eq!(truncate_long_text(&short).len(), LONG_TEXT_THRESHOLD);

        let over = "a".repeat(LONG_TEXT_THRESHOLD + 50);
        assert_eq!(truncate_long_text(&over).len(), LONG_TEXT_THRESHOLD);

        // Place a skin-toned emoji 👍🏽 (8 bytes: 4 + 4) straddling 2048.
        let mut s = "a".repeat(LONG_TEXT_THRESHOLD - 4);
        s.push_str("👍🏽after");
        let cut = truncate_long_text(&s);
        assert_eq!(cut.len(), LONG_TEXT_THRESHOLD - 4);
        assert!(!cut.ends_with('👍'));
    }

    #[test]
    fn a_transcript_reads_as_one_paragraph() {
        let whisper = " Bună ziua, salutări.\nVă transmit un mesaj vocal\n\n pentru  funcția de\ttranscriere.\n";
        assert_eq!(
            transcript_paragraph(whisper),
            "Bună ziua, salutări. Vă transmit un mesaj vocal pentru funcția de transcriere."
        );
        assert_eq!(transcript_paragraph(" \n "), "");
    }

    #[test]
    fn styled_parts_and_spoiler_masking() {
        let alice = ServiceId::Aci(Uuid::from_u128(1));
        let body = MessageBody::with_styles(
            "hi \u{FFFC} secret code",
            vec![
                StyleSpan {
                    start: 0,
                    len: 6,
                    style: TextStyle::Bold,
                },
                StyleSpan {
                    start: 3,
                    len: 3,
                    style: TextStyle::Italic,
                },
                StyleSpan {
                    start: 7,
                    len: 6,
                    style: TextStyle::Spoiler,
                },
                StyleSpan {
                    start: 14,
                    len: 4,
                    style: TextStyle::Monospace,
                },
            ],
        );
        let mentions = [Mention {
            who: alice,
            start: 3,
            len: 3,
        }];
        let parts = styled_parts(&body, &mentions);
        assert_eq!(
            parts,
            vec![
                StyledSegment::Text(
                    "hi ",
                    StyleFlags {
                        bold: true,
                        ..StyleFlags::default()
                    }
                ),
                StyledSegment::Mention(
                    alice,
                    StyleFlags {
                        bold: true,
                        italic: true,
                        ..StyleFlags::default()
                    }
                ),
                StyledSegment::Text(" ", StyleFlags::default()),
                StyledSegment::Text(
                    "secret",
                    StyleFlags {
                        spoiler: true,
                        ..StyleFlags::default()
                    }
                ),
                StyledSegment::Text(" ", StyleFlags::default()),
                StyledSegment::Text(
                    "code",
                    StyleFlags {
                        monospace: true,
                        ..StyleFlags::default()
                    }
                ),
            ]
        );
        assert_eq!(mask_spoilers(&body), "hi \u{FFFC} ░░░░░░ code");
    }
}
