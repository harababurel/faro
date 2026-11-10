//! faro's own domain types, modelled on Signal concepts rather than on presage
//! or signal-cli structs (DECISIONS D6, DESIGN.md §5.2).

mod ids;
mod message;
mod people;
mod presence;
mod sensitive;

pub use ids::{
    ConversationId, GroupId, MessageRef, ParseConversationIdError, ParseGroupIdError,
    ParseServiceIdError, ServiceId, Timestamp,
};
pub use message::{
    AttachmentId, AttachmentRef, AttachmentStatus, BodyPart, CallKind, CallState, DeliveryStatus,
    Direction, Mention, Message, Quote, Reaction, StyleFlags, StyleSpan, StyledSegment,
    SystemEvent, TextStyle, TranscriptSource, format_byte_size, format_disappearing_remaining,
    format_disappearing_timer, mask_spoilers, mention_parts, normalize_styles, styled_parts,
    transcript_paragraph,
};
pub use people::{
    Contact, Group, IdentityRecord, LinkedDevice, VerificationState, format_safety_number,
};
pub use presence::{ConnectionState, TypingState};
pub use sensitive::{MessageBody, PhoneNumber};
