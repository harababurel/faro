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
use crate::domain::{
    AttachmentId, AttachmentRef, AttachmentStatus, Contact, ConversationId, Group, GroupId,
    PhoneNumber, ServiceId,
};

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

pub(super) struct MockAttachment {
    pub(super) id: &'static str,
    pub(super) content_type: &'static str,
    pub(super) file_name: Option<&'static str>,
    pub(super) dims: Option<(u32, u32)>,
}

/// Canned attachment kinds.
pub(super) const ATTACHMENTS: &[MockAttachment] = &[
    MockAttachment {
        id: "mock:ridge-trail",
        content_type: "image/png",
        file_name: Some("ridge-trail.png"),
        dims: Some((64, 36)),
    },
    MockAttachment {
        id: "mock:crag-topo",
        content_type: "image/png",
        file_name: Some("crag-topo.png"),
        dims: Some((48, 48)),
    },
    MockAttachment {
        id: "mock:sticker-send",
        content_type: "image/png",
        file_name: None,
        dims: Some((32, 32)),
    },
    MockAttachment {
        id: "mock:gear-checklist",
        content_type: "text/plain",
        file_name: Some("gear-checklist.txt"),
        dims: None,
    },
];

/// Build a remote [`AttachmentRef`] for one of [`ATTACHMENTS`].
pub(super) fn mock_attachment(index: usize) -> Option<AttachmentRef> {
    let spec = ATTACHMENTS.get(index)?;
    let bytes = attachment_bytes(spec.id)?;
    Some(AttachmentRef {
        id: AttachmentId(spec.id.to_owned()),
        content_type: spec.content_type.to_owned(),
        file_name: spec.file_name.map(str::to_owned),
        size: Some(u64::try_from(bytes.len()).unwrap_or(0)),
        width: spec.dims.map(|(w, _)| w),
        height: spec.dims.map(|(_, h)| h),
        status: AttachmentStatus::Remote,
    })
}

/// Bytes served by [`super::MockBackend::fetch_attachment`] for `id`.
pub(super) fn attachment_bytes(id: &str) -> Option<Vec<u8>> {
    match id {
        "mock:ridge-trail" => Some(encode_rgb_png(64, 36, |x, y, w, h| {
            // Sunset sky over a dark mountain ridge with a bright trail line.
            let ridge = h / 2 + (x * 7).abs_diff(w * 3) / 6;
            if y < ridge {
                let t = u8::try_from((y * 255 / h.max(1)).min(255)).unwrap_or(0);
                [245u8.saturating_sub(t / 3), 120 + t / 3, 90 + t / 2]
            } else if y.abs_diff(ridge + (x / 5)) <= 1 {
                [250, 210, 90]
            } else {
                [36, 46, 66]
            }
        })),
        "mock:crag-topo" => Some(encode_rgb_png(48, 48, |x, y, w, h| {
            // Warm limestone face with a diagonal red route line.
            let diag = (x * h / w.max(1)).abs_diff(y);
            if diag <= 1 {
                [235, 75, 60]
            } else {
                let shade = u8::try_from((x * 3 + y * 5) % 28).unwrap_or(0);
                [165 + shade, 150 + shade, 130 + shade]
            }
        })),
        "mock:sticker-send" => Some(encode_rgb_png(32, 32, |x, y, w, h| {
            let dx = x.abs_diff(w / 2);
            let dy = y.abs_diff(h / 2);
            if dx * dx + dy * dy <= 130 {
                [80, 195, 150]
            } else {
                [28, 32, 42]
            }
        })),
        "mock:gear-checklist" => {
            Some(b"Rope 60m\n12 quickdraws\nChalk bag\nHeadlamp\nFirst-aid kit\n".to_vec())
        }
        _ => None,
    }
}

/// Minimal valid 8-bit RGB PNG using an uncompressed (stored) deflate block.
fn encode_rgb_png(
    width: u32,
    height: u32,
    pixel: impl Fn(u32, u32, u32, u32) -> [u8; 3],
) -> Vec<u8> {
    let mut raw = Vec::with_capacity((height * (1 + width * 3)) as usize);
    for y in 0..height {
        raw.push(0); // filter: None
        for x in 0..width {
            raw.extend_from_slice(&pixel(x, y, width, height));
        }
    }
    let len = u16::try_from(raw.len()).unwrap_or(u16::MAX);
    let mut zlib = Vec::with_capacity(raw.len() + 11);
    zlib.extend_from_slice(&[0x78, 0x01, 0x01]);
    zlib.extend_from_slice(&len.to_le_bytes());
    zlib.extend_from_slice(&(!len).to_le_bytes());
    zlib.extend_from_slice(&raw);
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);

    let mut png = Vec::with_capacity(zlib.len() + 64);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    write_png_chunk(&mut png, *b"IHDR", &ihdr);
    write_png_chunk(&mut png, *b"IDAT", &zlib);
    write_png_chunk(&mut png, *b"IEND", &[]);
    png
}

fn write_png_chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len()).unwrap_or(0);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(kind, data).to_be_bytes());
}

fn crc32(kind: [u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in kind.iter().chain(data) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
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

    #[test]
    fn canned_attachments_have_matching_bytes() {
        for i in 0..ATTACHMENTS.len() {
            let att = mock_attachment(i).expect("valid index");
            let bytes = attachment_bytes(&att.id.0).expect("bytes exist");
            assert_eq!(att.size, Some(bytes.len() as u64));
            if att.is_image() {
                assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            }
        }
    }
}
