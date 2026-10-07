//! faro's own domain types, modelled on Signal concepts rather than on presage
//! or signal-cli structs (DECISIONS D6, DESIGN.md §5.2).

mod ids;
mod message;
mod people;
mod presence;
mod sensitive;

pub use ids::{ConversationId, GroupId, MessageRef, ParseServiceIdError, ServiceId, Timestamp};
pub use message::{
    AttachmentId, AttachmentRef, DeliveryStatus, Direction, Message, Quote, Reaction,
};
pub use people::{Contact, Group};
pub use presence::{ConnectionState, TypingState};
pub use sensitive::{MessageBody, PhoneNumber};
