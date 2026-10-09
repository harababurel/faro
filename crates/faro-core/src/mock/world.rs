//! The demo's directory: the account, contacts and groups built from the cast
//! (`cast.rs`).
//!
//! Built from a constant seed, independent of [`MockConfig::seed`](super::MockConfig),
//! so IDs are the same in every run and history stored by one run (M2) still
//! matches the contacts of the next.

use super::cast::{CLIENTS, Chat, GROUPS, GroupKey, Members, PEOPLE, QUIET_CONTACTS, Who};
use crate::backend::Account;
use crate::domain::{Contact, ConversationId, Group, GroupId, PhoneNumber, ServiceId};

/// Seed for IDs. Changing it changes every mock ID, orphaning stored history.
const WORLD_SEED: u64 = 0x00fa_0001;

pub(super) struct World {
    pub(super) account: Account,
    pub(super) contacts: Vec<Contact>,
    pub(super) groups: Vec<Group>,
    /// Conversations that get scenes; the rest stay empty.
    pub(super) active: Vec<ConversationId>,
    /// Service IDs of the cast (not `Me`).
    cast: Vec<(Who, ServiceId)>,
    group_ids: Vec<(GroupKey, GroupId)>,
}

impl World {
    pub(super) fn me(&self) -> ServiceId {
        self.account.service_id
    }

    /// `who`'s service ID; the account's for `Me`.
    pub(super) fn id_of(&self, who: Who) -> Option<ServiceId> {
        if who == Who::Me {
            return Some(self.me());
        }
        self.cast.iter().find(|(w, _)| *w == who).map(|(_, id)| *id)
    }

    /// The cast member with service ID `id`, if any.
    pub(super) fn who_is(&self, id: ServiceId) -> Option<Who> {
        if id == self.me() {
            return Some(Who::Me);
        }
        self.cast.iter().find(|(_, s)| *s == id).map(|(w, _)| *w)
    }

    pub(super) fn conversation(&self, chat: Chat) -> Option<ConversationId> {
        match chat {
            Chat::Direct(who) => self.id_of(who).map(ConversationId::Direct),
            Chat::Group(key) => self
                .group_ids
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, id)| ConversationId::Group(*id)),
        }
    }

    /// People other than this account who can write in `conversation`.
    pub(super) fn others_in(&self, conversation: ConversationId) -> Vec<ServiceId> {
        let me = self.me();
        match conversation {
            ConversationId::Direct(id) => vec![id],
            ConversationId::Group(id) => self
                .groups
                .iter()
                .find(|g| g.id == id)
                .map(|g| g.members.iter().copied().filter(|m| *m != me).collect())
                .unwrap_or_default(),
        }
    }

    pub(super) fn has_group(&self, id: GroupId) -> bool {
        self.groups.iter().any(|g| g.id == id)
    }

    pub(super) fn build() -> Self {
        let mut ids = IdSource(fastrand::Rng::with_seed(WORLD_SEED));
        let account = Account {
            service_id: ids.service_id(),
            phone: Some(PhoneNumber::new("+15555550100")),
        };

        let mut phone_suffix = 101u32..;
        let mut phone = |has: bool| {
            has.then(|| phone_suffix.next())
                .flatten()
                .map(|n| PhoneNumber::new(format!("+1555555{n:04}")))
        };

        let mut cast = Vec::with_capacity(PEOPLE.len());
        let mut contacts = Vec::new();
        for person in PEOPLE {
            let service_id = ids.service_id();
            cast.push((person.who, service_id));
            contacts.push(Contact {
                service_id,
                name: person.name.map(Into::into),
                profile_name: person.profile.map(Into::into),
                phone: phone(person.phone),
            });
        }
        let quiet = QUIET_CONTACTS
            .iter()
            .map(|s| (*s).to_owned())
            .chain(CLIENTS.iter().map(|c| format!("Client – {c}")));
        contacts.extend(quiet.map(|name| Contact {
            service_id: ids.service_id(),
            name: Some(name),
            profile_name: None,
            phone: phone(true),
        }));

        let id_of = |who: Who| cast.iter().find(|(w, _)| *w == who).map(|(_, id)| *id);
        let mut groups = Vec::with_capacity(GROUPS.len());
        let mut group_ids = Vec::with_capacity(GROUPS.len());
        for spec in GROUPS {
            let members: Vec<ServiceId> = match spec.members {
                Members::These(people) => people.iter().filter_map(|w| id_of(*w)).collect(),
                Members::Everyone => contacts.iter().map(|c| c.service_id).collect(),
            };
            let id = ids.group_id();
            group_ids.push((spec.key, id));
            groups.push(Group {
                id,
                title: spec.title.to_owned(),
                members: members.into_iter().chain([account.service_id]).collect(),
            });
        }

        let direct = PEOPLE
            .iter()
            .filter(|p| p.direct)
            .filter_map(|p| id_of(p.who))
            .map(ConversationId::Direct);
        let group_chats = GROUPS
            .iter()
            .zip(&group_ids)
            .filter(|(spec, _)| spec.active)
            .map(|(_, (_, id))| ConversationId::Group(*id));
        let active = direct.chain(group_chats).collect();

        Self {
            account,
            contacts,
            groups,
            active,
            cast,
            group_ids,
        }
    }
}

/// Deterministic, valid-looking (v4) IDs.
struct IdSource(fastrand::Rng);

impl IdSource {
    fn service_id(&mut self) -> ServiceId {
        let mut bytes = [0u8; 16];
        self.0.fill(&mut bytes);
        ServiceId::Aci(uuid::Builder::from_random_bytes(bytes).into_uuid())
    }

    fn group_id(&mut self) -> GroupId {
        let mut bytes = [0u8; GroupId::LEN];
        self.0.fill(&mut bytes);
        GroupId::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use uuid::Uuid;

    use super::*;

    #[test]
    fn world_is_stable_and_consistent() {
        let a = World::build();
        let b = World::build();
        assert_eq!(a.account, b.account);
        assert_eq!(a.contacts, b.contacts);
        assert_eq!(a.groups, b.groups);

        let ids: HashSet<ServiceId> = a.contacts.iter().map(|c| c.service_id).collect();
        assert_eq!(ids.len(), a.contacts.len(), "contact IDs are unique");
        assert!(!ids.contains(&a.me()), "the account is not its own contact");
        assert_eq!(a.me().uuid().get_version_num(), 4);

        for g in &a.groups {
            assert!(g.members.contains(&a.me()), "{} includes me", g.title);
            assert!(g.members.iter().all(|m| *m == a.me() || ids.contains(m)));
        }
        for person in PEOPLE {
            let id = a.id_of(person.who).expect("everyone has an ID");
            assert_eq!(a.who_is(id), Some(person.who));
        }
        assert!(a.active.iter().all(|c| !a.others_in(*c).is_empty()));
    }

    #[test]
    fn uuids_are_not_nil() {
        assert_ne!(World::build().me().uuid(), Uuid::nil());
    }
}
