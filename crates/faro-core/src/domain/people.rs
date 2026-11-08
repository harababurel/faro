//! People, groups and linked devices.

use super::{GroupId, PhoneNumber, ServiceId, Timestamp};

/// A device linked to this Signal account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedDevice {
    pub id: u32,
    /// Decrypted name of the device (`None` for the primary phone or unnamed devices).
    pub name: Option<String>,
    pub created_at: Timestamp,
    pub last_seen: Timestamp,
    /// True when this entry is the current faro device.
    pub is_current: bool,
}

impl LinkedDevice {
    /// Display label for the device (`"Primary device"` for `id == 1` when
    /// unnamed, or `"Device #<id>"` when a secondary device has no name).
    pub fn display_name(&self) -> String {
        if let Some(name) = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return name.to_owned();
        }
        if self.id == 1 {
            "Primary device".to_owned()
        } else {
            format!("Device #{}", self.id)
        }
    }
}

/// A person faro knows about: a contact synced from the phone, or anyone seen in a
/// conversation (stored in the `recipients` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub service_id: ServiceId,
    /// Name from the phone's address book, if synced.
    pub name: Option<String>,
    /// Name the person set in their Signal profile.
    pub profile_name: Option<String>,
    pub phone: Option<PhoneNumber>,
}

impl Contact {
    /// The best name to show: address-book name, then profile name. `None` means the
    /// UI should fall back to the phone number or a placeholder.
    pub fn display_name(&self) -> Option<&str> {
        [&self.name, &self.profile_name]
            .into_iter()
            .flatten()
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: GroupId,
    pub title: String,
    pub members: Vec<ServiceId>,
}

/// Whether a contact's current identity key has been verified (D163).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VerificationState {
    #[default]
    Default,
    Verified,
    Unverified,
}

impl VerificationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Verified => "verified",
            Self::Unverified => "unverified",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "default" => Some(Self::Default),
            "verified" => Some(Self::Verified),
            "unverified" => Some(Self::Unverified),
            _ => None,
        }
    }
}

/// What faro knows about a contact's Signal identity key and safety number (D163).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityRecord {
    /// The 60-digit Signal safety number for the contact's current identity key.
    pub safety_number: Option<String>,
    /// Verification state of the current identity key.
    pub verified: VerificationState,
    /// True when the contact's safety number changed from an earlier stored one
    /// and the new key has not yet been verified or approved.
    pub key_changed: bool,
}

impl IdentityRecord {
    /// Apply a safety number and optional synced verification state from the
    /// backend. Returns `true` when this update replaced an earlier, different
    /// safety number without verifying the new one.
    pub fn update(&mut self, safety_number: String, verified: Option<VerificationState>) -> bool {
        let replaced = self
            .safety_number
            .as_deref()
            .is_some_and(|old| old != safety_number);
        self.safety_number = Some(safety_number);
        if replaced {
            if verified == Some(VerificationState::Verified) {
                self.verified = VerificationState::Verified;
                self.key_changed = false;
                false
            } else {
                self.key_changed = true;
                if let Some(v) = verified {
                    self.verified = v;
                } else if self.verified == VerificationState::Verified {
                    self.verified = VerificationState::Unverified;
                }
                true
            }
        } else {
            if let Some(v) = verified {
                self.verified = v;
                if v == VerificationState::Verified {
                    self.key_changed = false;
                }
            }
            false
        }
    }

    /// Record a user-chosen verification state (or approval of a changed key),
    /// clearing `key_changed`.
    pub fn set_verified(&mut self, verified: VerificationState) {
        self.verified = verified;
        self.key_changed = false;
    }
}

/// Format a 60-digit Signal safety number into 3 lines of 4 five-digit groups
/// (`"12345 67890 12345 67890"`).
pub fn format_safety_number(digits: &str) -> [String; 3] {
    let clean: String = digits.chars().filter(char::is_ascii_digit).collect();
    std::array::from_fn(|row| {
        (0..4)
            .filter_map(|col| {
                let start = (row * 4 + col) * 5;
                clean
                    .get(start..start + 5)
                    .or_else(|| clean.get(start..))
                    .filter(|s| !s.is_empty())
            })
            .collect::<Vec<_>>()
            .join(" ")
    })
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn contact(name: Option<&str>, profile: Option<&str>) -> Contact {
        Contact {
            service_id: ServiceId::Aci(Uuid::from_u128(1)),
            name: name.map(Into::into),
            profile_name: profile.map(Into::into),
            phone: Some(PhoneNumber::new("+15555550123")),
        }
    }

    #[test]
    fn display_name_prefers_address_book_and_skips_blank() {
        assert_eq!(
            contact(Some("Mum"), Some("Jane")).display_name(),
            Some("Mum")
        );
        assert_eq!(
            contact(Some("  "), Some("Jane")).display_name(),
            Some("Jane")
        );
        assert_eq!(contact(None, None).display_name(), None);
    }

    #[test]
    fn safety_number_formats_as_three_rows_of_four_groups() {
        let raw = "111112222233333444445555566666777778888899999000001234567890";
        assert_eq!(
            format_safety_number(raw),
            [
                "11111 22222 33333 44444",
                "55555 66666 77777 88888",
                "99999 00000 12345 67890",
            ]
        );
    }
}
