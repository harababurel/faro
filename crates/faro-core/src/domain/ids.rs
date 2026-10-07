//! Identifiers. Signal's stable IDs are the keys; phone numbers are display data
//! (DESIGN.md §5.2).

use std::fmt;
use std::str::FromStr;

use uuid::Uuid;

/// A Signal service ID: the ACI (account identity, the normal case) or the PNI
/// (phone-number identity, used before an ACI is known).
///
/// String form follows Signal: ACI is the bare UUID, PNI is `PNI:<uuid>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ServiceId {
    Aci(Uuid),
    Pni(Uuid),
}

impl ServiceId {
    pub fn uuid(self) -> Uuid {
        match self {
            Self::Aci(u) | Self::Pni(u) => u,
        }
    }
}

const PNI_PREFIX: &str = "PNI:";

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aci(u) => write!(f, "{u}"),
            Self::Pni(u) => write!(f, "{PNI_PREFIX}{u}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid service ID")]
pub struct ParseServiceIdError;

impl FromStr for ServiceId {
    type Err = ParseServiceIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parse = |u: &str| Uuid::try_parse(u).map_err(|_| ParseServiceIdError);
        match s.strip_prefix(PNI_PREFIX) {
            Some(rest) => parse(rest).map(Self::Pni),
            None => parse(s).map(Self::Aci),
        }
    }
}

/// A Signal group (v2) identifier: 32 bytes derived from the group master key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupId([u8; 32]);

impl GroupId {
    pub const LEN: usize = 32;

    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

/// Lowercase hex.
impl fmt::Display for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GroupId({self})")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid group ID")]
pub struct ParseGroupIdError;

/// Parses the [`Display`](fmt::Display) form: 64 hex digits (either case).
impl FromStr for GroupId {
    type Err = ParseGroupIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Checked up front: `from_str_radix` alone would accept a leading `+`.
        if s.len() != 2 * Self::LEN || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ParseGroupIdError);
        }
        let mut bytes = [0u8; Self::LEN];
        let (pairs, _) = s.as_bytes().as_chunks::<2>();
        for (byte, pair) in bytes.iter_mut().zip(pairs) {
            let pair = std::str::from_utf8(pair).map_err(|_| ParseGroupIdError)?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| ParseGroupIdError)?;
        }
        Ok(Self(bytes))
    }
}

/// A conversation is either with one person or with a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConversationId {
    Direct(ServiceId),
    Group(GroupId),
}

impl From<ServiceId> for ConversationId {
    fn from(id: ServiceId) -> Self {
        Self::Direct(id)
    }
}

impl From<GroupId> for ConversationId {
    fn from(id: GroupId) -> Self {
        Self::Group(id)
    }
}

const DIRECT_PREFIX: &str = "direct:";
const GROUP_PREFIX: &str = "group:";

/// `direct:<service id>` or `group:<hex>`: the database key and saved-state form.
impl fmt::Display for ConversationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct(peer) => write!(f, "{DIRECT_PREFIX}{peer}"),
            Self::Group(group) => write!(f, "{GROUP_PREFIX}{group}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid conversation ID")]
pub struct ParseConversationIdError;

impl FromStr for ConversationId {
    type Err = ParseConversationIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(peer) = s.strip_prefix(DIRECT_PREFIX) {
            peer.parse()
                .map(Self::Direct)
                .map_err(|_| ParseConversationIdError)
        } else if let Some(group) = s.strip_prefix(GROUP_PREFIX) {
            group
                .parse()
                .map(Self::Group)
                .map_err(|_| ParseConversationIdError)
        } else {
            Err(ParseConversationIdError)
        }
    }
}

/// Milliseconds since the Unix epoch, as Signal uses for message timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Timestamp(u64);

impl Timestamp {
    pub const fn from_millis(ms: u64) -> Self {
        Self(ms)
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }
}

/// Identifies a message the way Signal does: by its author and the sender's
/// `sent_at` timestamp. Used as the target of reactions, edits, deletes, receipts
/// and quotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageRef {
    pub sender: ServiceId,
    pub sent_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "6f0c4b2e-8d1a-4c3b-9e5f-0a1b2c3d4e5f";

    #[test]
    fn service_id_round_trips_signal_string_form() {
        let aci: ServiceId = U.parse().unwrap();
        assert!(matches!(aci, ServiceId::Aci(_)));
        assert_eq!(aci.to_string(), U);

        let pni: ServiceId = format!("PNI:{U}").parse().unwrap();
        assert!(matches!(pni, ServiceId::Pni(_)));
        assert_eq!(pni.to_string(), format!("PNI:{U}"));
        assert_eq!(aci.uuid(), pni.uuid());
        assert_ne!(aci, pni);
    }

    #[test]
    fn service_id_rejects_garbage() {
        assert!("".parse::<ServiceId>().is_err());
        assert!("PNI:".parse::<ServiceId>().is_err());
        assert!("ACI:nonsense".parse::<ServiceId>().is_err());
    }

    #[test]
    fn group_id_displays_as_hex() {
        let mut bytes = [0u8; GroupId::LEN];
        bytes[0] = 0xab;
        bytes[31] = 0x01;
        let id = GroupId::from_bytes(bytes);
        let hex = id.to_string();
        assert_eq!(hex.len(), 64);
        assert!(hex.starts_with("ab00"));
        assert!(hex.ends_with("0001"));
        assert_eq!(format!("{id:?}"), format!("GroupId({hex})"));
        assert_eq!(hex.parse::<GroupId>(), Ok(id));
        assert_eq!(hex.to_uppercase().parse::<GroupId>(), Ok(id));
    }

    #[test]
    fn group_id_rejects_garbage() {
        let ok = "ab".repeat(GroupId::LEN);
        assert!(ok.parse::<GroupId>().is_ok());
        assert!(ok[2..].parse::<GroupId>().is_err(), "too short");
        assert!(format!("{ok}00").parse::<GroupId>().is_err(), "too long");
        assert!(
            ok.replacen("ab", "zz", 1).parse::<GroupId>().is_err(),
            "not hex"
        );
        assert!(
            ok.replacen("ab", "+a", 1).parse::<GroupId>().is_err(),
            "sign"
        );
        assert!(
            ok.replacen("ab", "é", 1).parse::<GroupId>().is_err(),
            "non-ASCII"
        );
    }
}
