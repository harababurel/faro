//! Messages and what hangs off them.

use super::{ConversationId, MessageBody, MessageRef, ServiceId, Timestamp};

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
    pub attachments: Vec<AttachmentRef>,
    /// At most one per sender (Signal replaces a person's previous reaction).
    pub reactions: Vec<Reaction>,
    pub edited_at: Option<Timestamp>,
    /// Deleted for everyone; shown as a placeholder.
    pub deleted: bool,
}

impl Message {
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
}

/// `who` is mentioned in place of `body[start..start + len]` (byte offsets;
/// Signal puts U+FFFC there and the name is drawn instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mention {
    pub who: ServiceId,
    pub start: usize,
    pub len: usize,
}

/// A piece of a message body: text as written, or a mention to draw as a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyPart<'a> {
    Text(&'a str),
    Mention(ServiceId),
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

/// Metadata about an attachment; the content is fetched on demand (D75).
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

    /// The downloaded copy, if there is one.
    pub fn local_path(&self) -> Option<&std::path::Path> {
        match &self.status {
            AttachmentStatus::Downloaded(path) => Some(path),
            AttachmentStatus::Remote | AttachmentStatus::Failed => None,
        }
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
            attachments: vec![],
            reactions: vec![],
            edited_at: None,
            deleted: false,
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
    }
}
