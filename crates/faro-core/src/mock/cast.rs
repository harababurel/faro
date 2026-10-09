//! The demo's cast: an affectionate parody of *The Office* (US).
//!
//! This account is the new temp at the Scranton branch. It is never named:
//! characters call it "the temp" and mentions of it are drawn as `@You`.
//!
//! The cast is data only. [`World::build`](super::world::World::build) turns it
//! into contacts and groups, and `scenes.rs` gives everyone their lines. Between
//! them, the names cover what the UI has to cope with: a very long name, a ZWJ
//! emoji with a skin tone, a contact with no name at all, CJK, right-to-left
//! script, Devanagari, a group title too long for any sidebar, a group with more
//! than 40 members, and dozens of conversations that never see a message.

/// Someone who can write in a scene. `Me` is this account (the temp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum Who {
    Me,
    Michael,
    Dwight,
    Jim,
    Pam,
    Kelly,
    Oscar,
    Creed,
    Ryan,
    Andy,
    Kevin,
    Angela,
    Stanley,
    Tanaka,
    Mariam,
    KellysMom,
    // Quiet contacts: they never write to the temp directly, but talk in groups.
    Phyllis,
    Toby,
    Meredith,
}

/// A named member of the cast.
pub(super) struct Person {
    pub(super) who: Who,
    /// Name in this account's address book.
    pub(super) name: Option<&'static str>,
    /// Name set in their own Signal profile.
    pub(super) profile: Option<&'static str>,
    pub(super) phone: bool,
    /// Whether their direct chat with the temp gets scenes.
    pub(super) direct: bool,
}

const fn person(
    who: Who,
    name: Option<&'static str>,
    profile: Option<&'static str>,
    direct: bool,
) -> Person {
    Person {
        who,
        name,
        profile,
        phone: true,
        direct,
    }
}

/// Everyone in [`Who`] but `Me`, in ID order (changing the order changes IDs).
pub(super) const PEOPLE: &[Person] = &[
    person(
        Who::Michael,
        Some("Michael Scott"),
        Some("Michael Scott 🏆"),
        true,
    ),
    person(
        Who::Dwight,
        Some("Dwight K. Schrute III, Assistant (to the) Regional Manager"),
        Some("Dwight Schrute"),
        true,
    ),
    person(Who::Jim, Some("Jim Halpert"), Some("Jim"), true),
    person(Who::Pam, Some("Pam Beesly"), Some("Pam"), true),
    // No address-book name, so her profile name (and its many changes) shows.
    person(Who::Kelly, None, Some("Kelly Kapoor 💁🏽‍♀️"), true),
    person(Who::Oscar, Some("Oscar Martinez"), Some("Oscar"), true),
    // No name at all: the UI falls back to the phone number. It's Creed.
    person(Who::Creed, None, None, true),
    person(Who::Ryan, Some("Ryan Howard"), Some("Ryan"), true),
    person(Who::Andy, Some("Andy Bernard"), Some("Andy 🎶"), true),
    person(Who::Kevin, Some("Kevin Malone"), Some("Kevin"), true),
    person(Who::Angela, Some("Angela Martin"), Some("Angela"), true),
    person(Who::Stanley, Some("Stanley Hudson"), Some("Stanley"), true),
    person(
        Who::Tanaka,
        Some("Tanaka-san 🇯🇵 · supplier"),
        Some("田中 誠"),
        true,
    ),
    person(
        Who::Mariam,
        Some("Mariam · Cairo office supplies"),
        Some("مريم"),
        true,
    ),
    person(
        Who::KellysMom,
        Some("Kelly's mom"),
        Some("Mrs. Kapoor"),
        true,
    ),
    person(Who::Phyllis, Some("Phyllis Vance"), Some("Phyllis"), false),
    person(Who::Toby, Some("Toby Flenderson (HR)"), Some("Toby"), false),
    person(
        Who::Meredith,
        Some("Meredith Palmer"),
        Some("Meredith"),
        false,
    ),
];

/// Kelly's profile names, in the order she cycles through them.
pub(super) const KELLY_PROFILES: &[&str] = &[
    "Kelly Kapoor 💁🏽‍♀️",
    "Kelly 💅🏽✨ busy",
    "Kelly Kapoor 👑 business woman",
    "kelly 💔 do NOT text me ryan",
    "Kelly Kapoor 🪔 Diwali queen",
    "Kelly 🛍️💁🏽‍♀️ shopping is cardio",
];

/// Address-book entries that never see a message: the long tail of a real phone.
pub(super) const QUIET_CONTACTS: &[&str] = &[
    "Darryl Philbin (warehouse)",
    "Roy Anderson (warehouse)",
    "Erin Hannon",
    "Holly Flax",
    "Jan Levinson",
    "David Wallace (corporate)",
    "Gabe Lewis",
    "Nellie Bertram",
    "Mose Schrute",
    "Todd Packer",
    "Hank (security desk)",
    "Bob Vance, Vance Refrigeration",
    "Pizza by Alfredo",
    "Alfredo's Pizza Café",
    "Chili's",
    "Poor Richard's",
    "Scranton Business Park – building management",
];

/// Paper clients, listed as `Client – {name}`.
pub(super) const CLIENTS: &[&str] = &[
    "Lackawanna County Schools",
    "Scranton Public Library",
    "Electric City Realty",
    "Steamtown Print & Copy",
    "Pocono Family Dental",
    "Wilkes-Barre Law Offices",
    "Moosic Little League",
    "Clarks Summit Animal Hospital",
    "Dunmore Diner",
    "Nanticoke Credit Union",
    "Carbondale Chamber of Commerce",
    "Keystone Insurance Agency",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum GroupKey {
    Branch,
    PartyPlanning,
    FinerThings,
    Dundies,
    FateCommittee,
    BusinessPark,
}

pub(super) enum Members {
    /// These people and the temp.
    These(&'static [Who]),
    /// Every contact and the temp.
    Everyone,
}

pub(super) struct GroupSpec {
    pub(super) key: GroupKey,
    pub(super) title: &'static str,
    pub(super) members: Members,
    /// Whether the group gets scenes.
    pub(super) active: bool,
}

/// The office, minus the people who only write to the temp from outside it.
const BRANCH: &[Who] = &[
    Who::Michael,
    Who::Dwight,
    Who::Jim,
    Who::Pam,
    Who::Kelly,
    Who::Oscar,
    Who::Creed,
    Who::Ryan,
    Who::Andy,
    Who::Kevin,
    Who::Angela,
    Who::Stanley,
    Who::Phyllis,
    Who::Toby,
    Who::Meredith,
];

/// Groups in ID order (changing the order changes IDs).
pub(super) const GROUPS: &[GroupSpec] = &[
    GroupSpec {
        key: GroupKey::Branch,
        title: "Scranton Branch 📄",
        members: Members::These(BRANCH),
        active: true,
    },
    GroupSpec {
        key: GroupKey::PartyPlanning,
        title: "Party Planning Committee – OFFICIAL – do NOT add Michael 🚫🎉",
        members: Members::These(&[Who::Angela, Who::Pam, Who::Phyllis, Who::Kelly]),
        active: true,
    },
    GroupSpec {
        key: GroupKey::FinerThings,
        title: "Finer Things Club 🍷",
        members: Members::These(&[Who::Pam, Who::Oscar, Who::Toby]),
        active: true,
    },
    GroupSpec {
        key: GroupKey::Dundies,
        title: "Dundies 2026 🏆",
        members: Members::These(&[
            Who::Michael,
            Who::Dwight,
            Who::Jim,
            Who::Pam,
            Who::Kelly,
            Who::Kevin,
        ]),
        active: false,
    },
    GroupSpec {
        key: GroupKey::FateCommittee,
        title: "Committee to Determine the Fate of the Party Planning Committee",
        members: Members::These(&[Who::Angela, Who::Pam, Who::Phyllis, Who::Kelly, Who::Oscar]),
        active: false,
    },
    GroupSpec {
        key: GroupKey::BusinessPark,
        title: "Scranton Business Park",
        members: Members::Everyone,
        active: false,
    },
];

/// Where a scene takes place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Chat {
    /// The temp's direct chat with someone.
    Direct(Who),
    Group(GroupKey),
}
