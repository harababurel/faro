//! The mock's fixed cast: the account, contacts, groups and canned texts.
//!
//! Built from a constant seed, independent of [`MockConfig::seed`](super::MockConfig),
//! so IDs are the same in every run and history stored by one run (M2) still
//! matches the contacts of the next.
//!
//! The data is chosen to stress the UI: emoji (ZWJ sequences, flags, skin tones,
//! variation selectors), very long names, CJK, right-to-left text, combining marks,
//! contacts without any name, and many old conversations that never see a message
//! (to exercise conversation curation).

use crate::backend::Account;
use crate::domain::{Contact, ConversationId, Group, GroupId, PhoneNumber, ServiceId};

/// Seed for IDs. Changing it changes every mock ID, orphaning stored history.
const WORLD_SEED: u64 = 0x00fa_0001;

/// People who write messages. `(address-book name, profile name, has phone)`.
const ACTIVE_CONTACTS: &[(Option<&str>, Option<&str>, bool)] = &[
    (Some("Alice Moreau"), Some("Alice"), true),
    (Some("Bogdan 🚴‍♂️"), Some("Bogdan Ionescu"), true),
    (Some("陈伟 Chen Wei"), None, true),
    (None, Some("🌸🌸 Dana 🌸🌸"), false),
    (Some("Mum ❤️"), Some("Elena"), true),
    (
        Some("Dr. Bartholomew Maximilian Featherstonehaugh-Worthington III, Esq."),
        None,
        true,
    ),
    // Decomposed: "Zoe" + U+0308 COMBINING DIAERESIS.
    (Some("E\u{301}mile & Zoe\u{308} Ångström"), None, false),
    (None, Some("مريم"), true),
    (Some("Lukas 🇩🇪"), Some("Lukas"), true),
    // No name at all: the UI must fall back to the phone number.
    (None, None, true),
];

/// Contacts with no messages: the long tail every real address book has.
const QUIET_CONTACTS: &[&str] = &[
    "Plumber (2019)",
    "Airbnb host Lisbon",
    "Kevin from the conference",
    "Dentist",
    "Landlord – old flat",
    "Taxi 🚕 Cluj",
    "Hiking group organiser",
    "Bike shop",
    "Neighbour 4B",
    "Coworking front desk",
];

const FIRST_NAMES: &[&str] = &[
    "Ana", "Ben", "Clara", "David", "Eva", "Felix", "Greta", "Hugo", "Ines", "Jonas",
];
const LAST_NAMES: &[&str] = &["Pop", "Smith", "Novak", "García", "Kowalski"];

/// Canned message texts. Plain, emoji-heavy, long, multi-line, links, scripts.
pub(super) const TEXTS: &[&str] = &[
    "hey!",
    "ok",
    "👍",
    "😂😂😂",
    "on my way 🏃‍♀️💨",
    "did you see this? https://example.com/articles/2026/10/terminal-clients",
    "can we move it to 7?",
    "sounds good, see you there",
    "👨‍👩‍👧‍👦 dinner on sunday?",
    "🇷🇴 🇩🇪 🇯🇵 🏳️‍🌈 🏴‍☠️",
    "👋🏻👋🏼👋🏽👋🏾👋🏿",
    "❤️ ☺️ ✌️ (variation selectors)",
    "明天见！",
    "مرحبا، كيف حالك؟",
    "Zoe\u{308} says hi (combining marks)",
    "line one\nline two\nline three",
    "This is a deliberately long message to check wrapping. It goes on for a while, \
     mentions a few things, and keeps going past the width of any reasonable terminal \
     so that the message pane has to wrap it across several lines without breaking words \
     in the middle or losing the indentation of continuation lines.",
    "`cargo build --release` takes ages on the mini 🐢",
    "brb ☕",
    "🎉🎉🎉 congrats!!",
];

/// Emoji used for reactions (Signal's defaults plus a few multi-codepoint ones).
pub(super) const REACTIONS: &[&str] = &["❤️", "👍", "👎", "😂", "😮", "😢", "🙏", "👨‍💻", "🫶🏽"];

pub(super) struct World {
    pub(super) account: Account,
    pub(super) contacts: Vec<Contact>,
    pub(super) groups: Vec<Group>,
    /// Conversations that get scripted activity; the rest stay empty.
    pub(super) active: Vec<ConversationId>,
}

impl World {
    pub(super) fn me(&self) -> ServiceId {
        self.account.service_id
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

        let mut contacts: Vec<Contact> = ACTIVE_CONTACTS
            .iter()
            .map(|&(name, profile, has_phone)| Contact {
                service_id: ids.service_id(),
                name: name.map(Into::into),
                profile_name: profile.map(Into::into),
                phone: phone(has_phone),
            })
            .collect();
        let active_people: Vec<ServiceId> = contacts.iter().map(|c| c.service_id).collect();

        let generated = FIRST_NAMES
            .iter()
            .flat_map(|first| LAST_NAMES.iter().map(move |last| format!("{first} {last}")));
        let quiet = QUIET_CONTACTS
            .iter()
            .map(|s| (*s).to_owned())
            .chain(generated);
        contacts.extend(quiet.map(|name| Contact {
            service_id: ids.service_id(),
            name: Some(name),
            profile_name: None,
            phone: phone(true),
        }));

        let pick = |indices: &[usize]| -> Vec<ServiceId> {
            indices
                .iter()
                .filter_map(|&i| active_people.get(i).copied())
                .chain([account.service_id])
                .collect()
        };
        let everyone: Vec<ServiceId> = contacts
            .iter()
            .map(|c| c.service_id)
            .chain([account.service_id])
            .collect();

        let mut group = |title: &str, members: Vec<ServiceId>| Group {
            id: ids.group_id(),
            title: title.to_owned(),
            members,
        };
        let active_groups = vec![
            group("Family 👨‍👩‍👧‍👦", pick(&[4, 1, 0])),
            group("Climbing crew 🧗", pick(&[0, 1, 3, 8, 9])),
            group(
                "Neighbourhood association – autumn meeting planning committee 2026",
                pick(&[2, 5, 6, 7]),
            ),
        ];
        let quiet_groups = vec![
            group("Book club 📚", pick(&[0, 3, 6])),
            group("Old project (archived)", pick(&[2, 5])),
            group("Conference attendees", everyone),
        ];

        let active = active_people
            .iter()
            .map(|&id| ConversationId::Direct(id))
            .chain(active_groups.iter().map(|g| ConversationId::Group(g.id)))
            .collect();

        Self {
            account,
            contacts,
            groups: active_groups.into_iter().chain(quiet_groups).collect(),
            active,
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
    }

    #[test]
    fn world_stresses_the_ui() {
        let w = World::build();
        let names: Vec<&str> = w
            .contacts
            .iter()
            .filter_map(Contact::display_name)
            .collect();
        assert!(
            names.iter().any(|n| n.chars().count() > 40),
            "a very long name"
        );
        assert!(
            names.iter().any(|n| n.contains('\u{200d}')),
            "a ZWJ emoji name"
        );
        assert!(
            w.contacts.iter().any(|c| c.display_name().is_none()),
            "a nameless contact"
        );
        assert!(
            w.contacts.len() - w.active.len() >= 30,
            "many quiet conversations"
        );
        assert!(w.groups.len() > 3 && w.groups.iter().any(|g| g.members.len() > 40));
        assert!(w.active.iter().all(|c| !w.others_in(*c).is_empty()));
    }

    #[test]
    fn uuids_are_not_nil() {
        assert_ne!(World::build().me().uuid(), Uuid::nil());
    }
}
