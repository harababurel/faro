//! People and groups.

use super::{GroupId, PhoneNumber, ServiceId};

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
}
