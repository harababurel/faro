//! Conversions between domain types and their database representation.
//!
//! Every value read back is validated: a bad row becomes `StoreError::Corrupt`
//! naming the column, never a panic or a silently wrong value.

use super::{Result, StoreError};
use crate::domain::{ConversationId, DeliveryStatus, Direction, ServiceId, Timestamp};

const DIRECT: &str = "direct:";
const GROUP: &str = "group:";

/// `direct:<service id>` or `group:<hex>`.
pub(super) fn conversation_key(id: ConversationId) -> String {
    match id {
        ConversationId::Direct(peer) => format!("{DIRECT}{peer}"),
        ConversationId::Group(group) => format!("{GROUP}{group}"),
    }
}

pub(super) fn conversation_from_key(key: &str) -> Result<ConversationId> {
    let corrupt = || StoreError::Corrupt("conversation id");
    if let Some(peer) = key.strip_prefix(DIRECT) {
        peer.parse()
            .map(ConversationId::Direct)
            .map_err(|_| corrupt())
    } else if let Some(group) = key.strip_prefix(GROUP) {
        group
            .parse()
            .map(ConversationId::Group)
            .map_err(|_| corrupt())
    } else {
        Err(corrupt())
    }
}

pub(super) fn service_id_from_db(value: &str, column: &'static str) -> Result<ServiceId> {
    value.parse().map_err(|_| StoreError::Corrupt(column))
}

pub(super) fn timestamp_to_db(ts: Timestamp) -> Result<i64> {
    i64::try_from(ts.as_millis()).map_err(|_| StoreError::OutOfRange("timestamp"))
}

pub(super) fn timestamp_from_db(value: i64, column: &'static str) -> Result<Timestamp> {
    u64::try_from(value)
        .map(Timestamp::from_millis)
        .map_err(|_| StoreError::Corrupt(column))
}

pub(super) fn optional_timestamp_from_db(
    value: Option<i64>,
    column: &'static str,
) -> Result<Option<Timestamp>> {
    value.map(|v| timestamp_from_db(v, column)).transpose()
}

/// `(direction, status)` columns.
pub(super) fn direction_to_db(direction: Direction) -> (&'static str, Option<&'static str>) {
    match direction {
        Direction::Incoming => ("incoming", None),
        Direction::Outgoing(status) => ("outgoing", Some(status_to_db(status))),
    }
}

pub(super) fn direction_from_db(direction: &str, status: Option<&str>) -> Result<Direction> {
    match (direction, status) {
        ("incoming", None) => Ok(Direction::Incoming),
        ("outgoing", Some(status)) => status_from_db(status).map(Direction::Outgoing),
        _ => Err(StoreError::Corrupt("direction/status")),
    }
}

pub(super) fn status_to_db(status: DeliveryStatus) -> &'static str {
    match status {
        DeliveryStatus::Sending => "sending",
        DeliveryStatus::Sent => "sent",
        DeliveryStatus::Delivered => "delivered",
        DeliveryStatus::Read => "read",
        DeliveryStatus::Failed => "failed",
    }
}

pub(super) fn status_from_db(status: &str) -> Result<DeliveryStatus> {
    match status {
        "sending" => Ok(DeliveryStatus::Sending),
        "sent" => Ok(DeliveryStatus::Sent),
        "delivered" => Ok(DeliveryStatus::Delivered),
        "read" => Ok(DeliveryStatus::Read),
        "failed" => Ok(DeliveryStatus::Failed),
        _ => Err(StoreError::Corrupt("status")),
    }
}

pub(super) fn count_from_db(value: i64, column: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| StoreError::Corrupt(column))
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::GroupId;

    #[test]
    fn conversation_keys_round_trip() {
        let ids = [
            ConversationId::Direct(ServiceId::Aci(Uuid::from_u128(1))),
            ConversationId::Direct(ServiceId::Pni(Uuid::from_u128(2))),
            ConversationId::Group(GroupId::from_bytes([7; GroupId::LEN])),
        ];
        for id in ids {
            assert_eq!(conversation_from_key(&conversation_key(id)).unwrap(), id);
        }
        assert!(conversation_from_key("direct:nonsense").is_err());
        assert!(conversation_from_key("group:abc").is_err());
        assert!(conversation_from_key("channel:x").is_err());
    }

    #[test]
    fn directions_round_trip() {
        let all = [
            Direction::Incoming,
            Direction::Outgoing(DeliveryStatus::Sending),
            Direction::Outgoing(DeliveryStatus::Sent),
            Direction::Outgoing(DeliveryStatus::Delivered),
            Direction::Outgoing(DeliveryStatus::Read),
            Direction::Outgoing(DeliveryStatus::Failed),
        ];
        for d in all {
            let (direction, status) = direction_to_db(d);
            assert_eq!(direction_from_db(direction, status).unwrap(), d);
        }
        assert!(direction_from_db("incoming", Some("read")).is_err());
        assert!(direction_from_db("outgoing", None).is_err());
    }

    #[test]
    fn timestamps_are_range_checked() {
        assert!(timestamp_to_db(Timestamp::from_millis(u64::MAX)).is_err());
        assert!(timestamp_from_db(-1, "sent_at").is_err());
        let ts = Timestamp::from_millis(1_767_225_600_000);
        assert_eq!(
            timestamp_from_db(timestamp_to_db(ts).unwrap(), "x").unwrap(),
            ts
        );
    }
}
