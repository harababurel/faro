//! Conversions between domain types and their database representation.
//!
//! Every value read back is validated: a bad row becomes `StoreError::Corrupt`
//! naming the column, never a panic or a silently wrong value.
//!
//! The memory engine (`mem.rs`) uses only the keys and range checks, so
//! without SQLite most of this is unused.
#![cfg_attr(
    not(feature = "sqlite"),
    expect(
        dead_code,
        reason = "the read-back codecs serve only the SQLite engine"
    )
)]

use std::path::PathBuf;

use super::{Result, StoreError};
use crate::domain::{
    AttachmentStatus, CallKind, CallState, ConversationId, DeliveryStatus, Direction, ServiceId,
    SystemEvent, TextStyle, Timestamp, TranscriptSource, VerificationState,
};

/// `direct:<service id>` or `group:<hex>` (`ConversationId`'s string form).
pub(super) fn conversation_key(id: ConversationId) -> String {
    id.to_string()
}

pub(super) fn conversation_from_key(key: &str) -> Result<ConversationId> {
    key.parse()
        .map_err(|_| StoreError::Corrupt("conversation id"))
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

/// `(status, local_path)` columns of `attachments` (D75).
pub(super) fn attachment_status_to_db(
    status: &AttachmentStatus,
) -> Result<(&'static str, Option<&str>)> {
    match status {
        AttachmentStatus::Remote => Ok(("remote", None)),
        AttachmentStatus::Downloaded(path) => {
            let text = path
                .to_str()
                .filter(|s| !s.is_empty())
                .ok_or(StoreError::OutOfRange("attachment path"))?;
            Ok(("downloaded", Some(text)))
        }
        AttachmentStatus::Failed => Ok(("failed", None)),
    }
}

pub(super) fn attachment_status_from_db(
    status: &str,
    local_path: Option<String>,
) -> Result<AttachmentStatus> {
    match (status, local_path) {
        ("remote", _) => Ok(AttachmentStatus::Remote),
        ("downloaded", Some(path)) if !path.is_empty() => {
            Ok(AttachmentStatus::Downloaded(PathBuf::from(path)))
        }
        ("failed", _) => Ok(AttachmentStatus::Failed),
        _ => Err(StoreError::Corrupt("attachment status")),
    }
}

pub(super) fn count_from_db(value: i64, column: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| StoreError::Corrupt(column))
}

pub(super) fn transcript_source_to_db(source: TranscriptSource) -> &'static str {
    match source {
        TranscriptSource::Local => "local",
        TranscriptSource::Remote => "remote",
        TranscriptSource::Demo => "demo",
    }
}

pub(super) fn transcript_source_from_db(source: &str) -> Result<TranscriptSource> {
    match source {
        "local" => Ok(TranscriptSource::Local),
        "remote" => Ok(TranscriptSource::Remote),
        "demo" => Ok(TranscriptSource::Demo),
        _ => Err(StoreError::Corrupt("transcript source")),
    }
}

pub(super) fn verification_from_db(value: &str) -> Result<VerificationState> {
    VerificationState::parse(value).ok_or(StoreError::Corrupt("recipients.verified"))
}

pub(super) fn text_style_from_db(value: &str) -> Result<TextStyle> {
    TextStyle::parse(value).ok_or(StoreError::Corrupt("message_styles.style"))
}

pub(super) fn system_event_to_db(event: &SystemEvent) -> String {
    match event {
        SystemEvent::DisappearingTimer { seconds } => {
            format!("timer:{}", seconds.unwrap_or(0))
        }
        SystemEvent::GroupRenamed { title } => format!("rename:{title}"),
        SystemEvent::MembersJoined { members } => {
            let list: Vec<String> = members.iter().map(ToString::to_string).collect();
            format!("joined:{}", list.join(","))
        }
        SystemEvent::MembersLeft { members } => {
            let list: Vec<String> = members.iter().map(ToString::to_string).collect();
            format!("left:{}", list.join(","))
        }
        SystemEvent::Call { kind, state } => {
            let k = match kind {
                CallKind::Voice => "voice",
                CallKind::Video => "video",
                CallKind::Group => "group",
            };
            let s = match state {
                CallState::Incoming => "incoming",
                CallState::Missed => "missed",
                CallState::Accepted => "accepted",
            };
            format!("call:{k}:{s}")
        }
    }
}

pub(super) fn system_event_from_db(value: &str) -> Result<SystemEvent> {
    if let Some(rest) = value.strip_prefix("timer:") {
        let secs: u32 = rest
            .parse()
            .map_err(|_| StoreError::Corrupt("messages.system_event"))?;
        return Ok(SystemEvent::DisappearingTimer {
            seconds: (secs > 0).then_some(secs),
        });
    }
    if let Some(title) = value.strip_prefix("rename:") {
        return Ok(SystemEvent::GroupRenamed {
            title: title.to_owned(),
        });
    }
    if let Some(rest) = value.strip_prefix("joined:") {
        let members = parse_member_list(rest)?;
        return Ok(SystemEvent::MembersJoined { members });
    }
    if let Some(rest) = value.strip_prefix("left:") {
        let members = parse_member_list(rest)?;
        return Ok(SystemEvent::MembersLeft { members });
    }
    if let Some(rest) = value.strip_prefix("call:") {
        let (kind_str, state_str) = rest
            .split_once(':')
            .ok_or(StoreError::Corrupt("messages.system_event"))?;
        let kind = match kind_str {
            "voice" => CallKind::Voice,
            "video" => CallKind::Video,
            "group" => CallKind::Group,
            _ => return Err(StoreError::Corrupt("messages.system_event")),
        };
        let state = match state_str {
            "incoming" => CallState::Incoming,
            "missed" => CallState::Missed,
            "accepted" => CallState::Accepted,
            _ => return Err(StoreError::Corrupt("messages.system_event")),
        };
        return Ok(SystemEvent::Call { kind, state });
    }
    Err(StoreError::Corrupt("messages.system_event"))
}

fn parse_member_list(rest: &str) -> Result<Vec<ServiceId>> {
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    rest.split(',')
        .map(|s| service_id_from_db(s, "messages.system_event"))
        .collect()
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

    #[test]
    fn attachment_statuses_round_trip() {
        let statuses = [
            AttachmentStatus::Remote,
            AttachmentStatus::Downloaded(PathBuf::from("/tmp/faro/a.jpg")),
            AttachmentStatus::Failed,
        ];
        for s in statuses {
            let (status, path) = attachment_status_to_db(&s).unwrap();
            assert_eq!(
                attachment_status_from_db(status, path.map(str::to_owned)).unwrap(),
                s
            );
        }
        assert!(attachment_status_from_db("downloaded", None).is_err());
        assert!(attachment_status_from_db("unknown", None).is_err());
    }

    #[test]
    fn system_events_round_trip() {
        let a = ServiceId::Aci(Uuid::from_u128(1));
        let b = ServiceId::Pni(Uuid::from_u128(2));
        let events = [
            SystemEvent::DisappearingTimer { seconds: None },
            SystemEvent::DisappearingTimer {
                seconds: Some(3600),
            },
            SystemEvent::GroupRenamed {
                title: "Scranton: Finer Things".into(),
            },
            SystemEvent::MembersJoined {
                members: vec![a, b],
            },
            SystemEvent::MembersLeft { members: vec![a] },
            SystemEvent::Call {
                kind: CallKind::Voice,
                state: CallState::Incoming,
            },
            SystemEvent::Call {
                kind: CallKind::Video,
                state: CallState::Missed,
            },
            SystemEvent::Call {
                kind: CallKind::Group,
                state: CallState::Accepted,
            },
        ];
        for e in events {
            let encoded = system_event_to_db(&e);
            assert_eq!(system_event_from_db(&encoded).unwrap(), e);
        }
        assert!(system_event_from_db("unknown:1").is_err());
        assert!(system_event_from_db("timer:abc").is_err());
        assert!(system_event_from_db("call:voice: weird").is_err());
    }
}
