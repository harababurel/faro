//! Applying backend events to the open conversation's messages, with the same
//! rules the store applies to its rows (D34): idempotent, receipts never
//! regress, only newer edits win, deletes drop content.

use faro_core::backend::BackendEvent;
use faro_core::domain::{Direction, Message, MessageRef};

/// Apply `event` to `messages` (oldest first). Events about messages that aren't
/// loaded are ignored.
pub fn apply(messages: &mut Vec<Message>, event: &BackendEvent) {
    match event {
        BackendEvent::Message(message) => insert(messages, message),
        BackendEvent::DeliveryUpdate { message, status } => {
            if let Some(m) = find(messages, *message)
                && let Direction::Outgoing(current) = m.direction
            {
                m.direction = Direction::Outgoing(current.update(*status));
            }
        }
        BackendEvent::Reaction {
            target,
            sender,
            emoji,
            ..
        } => {
            if let Some(m) = find(messages, *target)
                && !m.deleted
            {
                m.apply_reaction(*sender, emoji.clone());
            }
        }
        BackendEvent::Edited {
            target,
            body,
            edited_at,
            ..
        } => {
            if let Some(m) = find(messages, *target)
                && !m.deleted
                && m.edited_at.is_none_or(|at| at < *edited_at)
            {
                m.body = Some(body.clone());
                m.edited_at = Some(*edited_at);
            }
        }
        BackendEvent::Deleted { target, .. } => {
            if let Some(m) = find(messages, *target) {
                m.deleted = true;
                m.body = None;
                if let Some(quote) = &mut m.quote {
                    quote.preview = None;
                }
                m.reactions.clear();
                m.attachments.clear();
            }
        }
        BackendEvent::Connection(_)
        | BackendEvent::Typing { .. }
        | BackendEvent::ContactUpdated(_)
        | BackendEvent::GroupUpdated(_)
        | BackendEvent::Error(_) => {}
    }
}

/// Insert in `(sent_at, sender)` order; a duplicate keeps the first copy.
fn insert(messages: &mut Vec<Message>, message: &Message) {
    let key = |m: &Message| (m.sent_at, m.sender);
    let at = messages.partition_point(|m| key(m) < key(message));
    if messages.get(at).is_some_and(|m| key(m) == key(message)) {
        return;
    }
    messages.insert(at, message.clone());
}

/// Recent messages are the likeliest targets, so search from the end.
fn find(messages: &mut [Message], target: MessageRef) -> Option<&mut Message> {
    messages
        .iter_mut()
        .rev()
        .find(|m| m.sent_at == target.sent_at && m.sender == target.sender)
}

#[cfg(test)]
mod tests {
    use faro_core::domain::{
        ConversationId, DeliveryStatus, MessageBody, Reaction, ServiceId, Timestamp,
    };
    use uuid::Uuid;

    use super::*;

    fn aci(n: u128) -> ServiceId {
        ServiceId::Aci(Uuid::from_u128(n))
    }

    fn message(sender: u128, at: u64, direction: Direction) -> Message {
        Message {
            conversation: ConversationId::Direct(aci(2)),
            sender: aci(sender),
            sent_at: Timestamp::from_millis(at),
            direction,
            body: Some(MessageBody::new("hi")),
            quote: None,
            attachments: vec![],
            reactions: vec![],
            edited_at: None,
            deleted: false,
        }
    }

    fn times(messages: &[Message]) -> Vec<u64> {
        messages.iter().map(|m| m.sent_at.as_millis()).collect()
    }

    #[test]
    fn inserts_in_order_without_duplicates() {
        let mut messages = vec![];
        for at in [30, 10, 20, 10] {
            apply(
                &mut messages,
                &BackendEvent::Message(message(2, at, Direction::Incoming)),
            );
        }
        assert_eq!(times(&messages), vec![10, 20, 30]);
    }

    #[test]
    fn receipts_edits_reactions_and_deletes() {
        let mine = message(1, 10, Direction::Outgoing(DeliveryStatus::Sent));
        let theirs = message(2, 20, Direction::Incoming);
        let mut messages = vec![mine.clone(), theirs.clone()];

        for status in [DeliveryStatus::Read, DeliveryStatus::Delivered] {
            apply(
                &mut messages,
                &BackendEvent::DeliveryUpdate {
                    message: mine.reference(),
                    status,
                },
            );
        }
        assert_eq!(
            messages[0].direction,
            Direction::Outgoing(DeliveryStatus::Read)
        );

        let edit = |body: &str, at| BackendEvent::Edited {
            conversation: theirs.conversation,
            target: theirs.reference(),
            body: MessageBody::new(body),
            edited_at: Timestamp::from_millis(at),
        };
        apply(&mut messages, &edit("new", 50));
        apply(&mut messages, &edit("old", 40));
        assert_eq!(messages[1].body, Some(MessageBody::new("new")));

        apply(
            &mut messages,
            &BackendEvent::Reaction {
                conversation: theirs.conversation,
                target: theirs.reference(),
                sender: aci(1),
                emoji: Some("👍".into()),
            },
        );
        assert_eq!(
            messages[1].reactions,
            vec![Reaction {
                sender: aci(1),
                emoji: "👍".into()
            }]
        );

        apply(
            &mut messages,
            &BackendEvent::Deleted {
                conversation: theirs.conversation,
                target: theirs.reference(),
            },
        );
        apply(&mut messages, &edit("zombie", 60));
        assert!(messages[1].deleted);
        assert_eq!(messages[1].body, None);
        assert_eq!(messages[1].reactions, vec![]);
    }
}
