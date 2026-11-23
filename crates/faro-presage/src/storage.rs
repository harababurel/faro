//! Contacts and groups from Signal's storage service (D179).
//!
//! The phone's contact sync carries only address-book names and no profile
//! keys. Signal's own apps read everything else (profile keys, profile
//! names, the nicknames set in the app) from the storage service, the
//! account's encrypted state on Signal's servers. Without a profile key a
//! person's profile can't be fetched, and they'd show as the start of their
//! ID. Groups are listed there too: otherwise faro only learns of a group
//! when a message arrives in it. faro only reads the storage service, never
//! writes it.

use faro_core::domain::ServiceId;
use presage::libsignal_service::StorageService;
use presage::libsignal_service::master_key::StorageServiceKey;
use presage::libsignal_service::proto::manifest_record::identifier::Type;
use presage::libsignal_service::proto::{ContactRecord, storage_record};
use presage::store::StateStore as _;
use uuid::Uuid;

use crate::Linked;

/// Signal's apps read at most this many items in one request.
const READ_BATCH: usize = 1000;

/// What the storage service says about one person.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct StoredContact {
    /// The name the user gave them: the nickname set in Signal, else the
    /// phone's address-book name.
    pub name: Option<String>,
    /// Their profile name as the phone last saw it.
    pub profile_name: Option<String>,
    /// Phone number in E.164 form.
    pub phone: Option<String>,
    pub profile_key: Option<[u8; 32]>,
}

/// What the storage service holds that faro uses.
#[derive(Debug, Default)]
pub(crate) struct Stored {
    pub contacts: Vec<(ServiceId, StoredContact)>,
    /// Master keys of the groups the account is in (blocked ones left out).
    pub groups: Vec<[u8; 32]>,
}

/// Read every contact and group record. Network; run it off the receive
/// loop.
pub(crate) async fn read(linked: &Linked) -> Result<Stored, String> {
    let master_key = linked
        .store()
        .fetch_master_key()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("this link has no storage key")?;
    let service = crate::transfer::build_push_service(linked).map_err(|e| e.to_string())?;
    let storage = StorageService::new(service, StorageServiceKey::from_master_key(&master_key))
        .await
        .map_err(|e| e.to_string())?;
    let manifest = storage.manifest().await.map_err(|e| e.to_string())?;
    let keys: Vec<Vec<u8>> = manifest
        .identifiers
        .iter()
        .filter(|id| id.r#type == Type::Contact as i32 || id.r#type == Type::Groupv2 as i32)
        .map(|id| id.raw.clone())
        .collect();
    let record_ikm = (!manifest.record_ikm.is_empty()).then_some(manifest.record_ikm.as_slice());
    let mut stored = Stored::default();
    for batch in keys.chunks(READ_BATCH) {
        let records = storage
            .read_items(batch.to_vec(), record_ikm)
            .await
            .map_err(|e| e.to_string())?;
        for record in records {
            match record.record {
                Some(storage_record::Record::Contact(contact)) => {
                    stored.contacts.extend(from_record(&contact));
                }
                Some(storage_record::Record::GroupV2(group)) if !group.blocked => {
                    stored
                        .groups
                        .extend(<[u8; 32]>::try_from(group.master_key.as_slice()).ok());
                }
                _ => {}
            }
        }
    }
    Ok(stored)
}

/// One record as faro needs it. `None` for people without an ACI.
pub(crate) fn from_record(record: &ContactRecord) -> Option<(ServiceId, StoredContact)> {
    let uuid = Uuid::from_slice(&record.aci_binary)
        .ok()
        .or_else(|| Uuid::parse_str(&record.aci).ok())?;
    let nickname = record
        .nickname
        .as_ref()
        .and_then(|n| join(&n.given, &n.family));
    let system = join(&record.system_given_name, &record.system_family_name)
        .or_else(|| join(&record.system_nickname, ""));
    Some((
        ServiceId::Aci(uuid),
        StoredContact {
            name: nickname.or(system),
            profile_name: join(&record.given_name, &record.family_name),
            phone: Some(record.e164.trim().to_owned()).filter(|p| !p.is_empty()),
            profile_key: <[u8; 32]>::try_from(record.profile_key.as_slice()).ok(),
        },
    ))
}

/// "Given Family", leaving out empty parts.
fn join(given: &str, family: &str) -> Option<String> {
    let name = [given.trim(), family.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use presage::libsignal_service::proto::contact_record::Name;

    use super::*;

    const ID: Uuid = Uuid::from_u128(7);

    fn record() -> ContactRecord {
        ContactRecord {
            aci: ID.to_string(),
            ..ContactRecord::default()
        }
    }

    #[test]
    fn a_nickname_wins_over_the_address_book_name() {
        let mut both = record();
        both.nickname = Some(Name {
            given: "Kev".to_owned(),
            family: String::new(),
        });
        both.system_given_name = "Kevin".to_owned();
        both.system_family_name = "Malone".to_owned();
        let (id, found) = from_record(&both).unwrap();
        assert_eq!(id, ServiceId::Aci(ID));
        assert_eq!(found.name.as_deref(), Some("Kev"));

        both.nickname = None;
        let (_, found) = from_record(&both).unwrap();
        assert_eq!(found.name.as_deref(), Some("Kevin Malone"));
    }

    #[test]
    fn profile_name_key_and_phone_are_read() {
        let mut full = record();
        full.given_name = " Oscar ".to_owned();
        full.family_name = "Martinez".to_owned();
        full.e164 = "+15555550123".to_owned();
        full.profile_key = vec![9; 32];
        let (_, found) = from_record(&full).unwrap();
        assert_eq!(found.name, None);
        assert_eq!(found.profile_name.as_deref(), Some("Oscar Martinez"));
        assert_eq!(found.phone.as_deref(), Some("+15555550123"));
        assert_eq!(found.profile_key, Some([9; 32]));
    }

    #[test]
    fn empty_fields_and_bad_keys_are_left_out() {
        let mut bare = record();
        bare.profile_key = vec![1; 5];
        let (_, found) = from_record(&bare).unwrap();
        assert_eq!(found, StoredContact::default());
    }

    #[test]
    fn the_binary_id_is_preferred_and_records_without_one_are_skipped() {
        let mut binary = ContactRecord::default();
        assert_eq!(from_record(&binary), None);
        binary.aci_binary = ID.as_bytes().to_vec();
        assert_eq!(
            from_record(&binary).map(|(id, _)| id),
            Some(ServiceId::Aci(ID))
        );
    }
}
