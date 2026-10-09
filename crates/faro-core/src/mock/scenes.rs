//! What the demo's cast says: scenes, reply pools and favourite reactions.
//!
//! A [`Scene`] is a short written conversation in one chat. The simulator plays
//! scenes line by line (`sim.rs`): about a dozen as the backlog, then one line
//! per scripted "message" activity, with typing indicators. Reactions, edits and
//! deletes written into a line follow it as their own events.
//!
//! Writing rules: original lines only, in each character's voice (short
//! catchphrases are fine); no trademarked company name; the overseas characters
//! are written respectfully, in correct Japanese, Arabic and Hindi, and each of
//! their scenes has a short English line so the demo stays readable.

use super::assets::Asset;
use super::cast::{Chat, GroupKey, Who};

/// A short conversation in one chat.
pub(super) struct Scene {
    /// For tests and debug logs; never shown.
    pub(super) title: &'static str,
    pub(super) chat: Chat,
    pub(super) lines: &'static [Line],
}

/// One message in a scene.
pub(super) struct Line {
    /// `Who::Me` lines arrive as messages "sent from the temp's phone".
    pub(super) who: Who,
    /// May be empty when the line is only an attachment.
    pub(super) text: &'static str,
    pub(super) attach: Option<Asset>,
    /// Index of an earlier line in this scene that this one quotes.
    pub(super) reply_to: Option<u8>,
    /// Someone mentioned at the start of the message (drawn as `@Name`).
    pub(super) mention: Option<Who>,
    pub(super) reactions: &'static [(Who, &'static str)],
    pub(super) then: Then,
}

/// What happens to a line after it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Then {
    Stay,
    /// The sender edits it to this text (the mention, if any, stays).
    EditTo(&'static str),
    /// The sender deletes it for everyone.
    Delete,
}

/// A plain line; the methods below add the rest.
const fn say(who: Who, text: &'static str) -> Line {
    Line {
        who,
        text,
        attach: None,
        reply_to: None,
        mention: None,
        reactions: &[],
        then: Then::Stay,
    }
}

impl Line {
    const fn with(mut self, asset: Asset) -> Self {
        self.attach = Some(asset);
        self
    }

    const fn reply(mut self, to: u8) -> Self {
        self.reply_to = Some(to);
        self
    }

    const fn at(mut self, who: Who) -> Self {
        self.mention = Some(who);
        self
    }

    const fn react(mut self, reactions: &'static [(Who, &'static str)]) -> Self {
        self.reactions = reactions;
        self
    }

    const fn edit(mut self, to: &'static str) -> Self {
        self.then = Then::EditTo(to);
        self
    }

    const fn delete(mut self) -> Self {
        self.then = Then::Delete;
        self
    }
}

use Who::{
    Andy, Angela, Creed, Dwight, Jim, Kelly, KellysMom, Kevin, Mariam, Me, Meredith, Michael,
    Oscar, Pam, Phyllis, Ryan, Stanley, Tanaka, Toby,
};

pub(super) const SCENES: &[Scene] = &[
    Scene {
        title: "Michael's welcome voice note",
        chat: Chat::Direct(Michael),
        lines: &[
            say(Michael, "Welcome to the family!!! ☺️ ✌️").react(&[(Me, "❤️")]),
            say(Michael, "").with(Asset::MichaelVoice),
            say(
                Michael,
                "That was me. Michael. Your boss. But also your friend.\n\
                 Mostly friend.\n\
                 Boss when it matters.",
            ),
            say(Me, "Thank you! Happy to be here 🙂"),
            say(Michael, "Here is a picture of my abs 😏📸").delete(),
            say(Michael, "Wrong chat. Do not tell Toby."),
        ],
    },
    Scene {
        title: "Dwight's security questionnaire",
        chat: Chat::Direct(Dwight),
        lines: &[
            say(
                Dwight,
                "Temp. Security protocol requires that you complete the attached \
                 checklist before 0900 hours.",
            )
            .with(Asset::EmergencyProtocol),
            say(Me, "Is this from HR?"),
            say(Dwight, "HR is a waste of a building. This is from me.").reply(1),
            say(
                Dwight,
                "Question one. Who is your favourite Battlestar Galactica character, \
                 and why is it Starbuck?",
            ),
            say(Me, "I haven't seen it, sorry"),
            say(Dwight, "Unbelievable. Gott im Himmel. 🇩🇪").react(&[(Me, "😮")]),
        ],
    },
    Scene {
        title: "Jim recruits the temp for the jello prank",
        chat: Chat::Direct(Jim),
        lines: &[
            say(Jim, "Hey. Quick question. How do you feel about jello?"),
            say(Me, "...positive?"),
            say(Jim, "Great. Dwight's at the dentist until 2.").with(Asset::Stapler),
            say(Jim, "I need someone to keep watch by the copier 👀"),
            say(Me, "I'm in"),
            say(Jim, "Welcome to the team.")
                .reply(4)
                .react(&[(Me, "😂")]),
        ],
    },
    Scene {
        title: "Fire drill",
        chat: Chat::Group(GroupKey::Branch),
        lines: &[
            say(
                Dwight,
                "ATTENTION. Unannounced fire drill at 1500 hours. This is the announcement.",
            ),
            say(Oscar, "You can't announce an unannounced drill, Dwight.")
                .edit("You can't announce an unannounced drill, Dwight. Por favor.")
                .react(&[(Jim, "😂"), (Angela, "👎")]),
            say(
                Dwight,
                "you are in charge of the fire extinguisher. Do NOT lose it.",
            )
            .at(Me),
            say(Me, "Where is it?"),
            say(Stanley, "I'm not moving for a drill.").reply(0),
        ],
    },
];

/// Lines for answering the temp's messages, by sender.
pub(super) const REPLIES: &[(Who, &[&str])] = &[
    (
        Michael,
        &[
            "That's what she said! 😂",
            "Love it. Love YOU. Professionally.",
            "Let's circle back on that after my nap. I mean my meeting.",
            "I am Beyoncé, always. 💅",
        ],
    ),
    (
        Dwight,
        &[
            "Question. Why are you messaging me during work hours?",
            "False.",
            "Noted. Your file has been updated.",
        ],
    ),
    (Jim, &["Ha. Nice.", "😐", "I'll allow it."]),
    (
        Pam,
        &[
            "Aw, thanks! 😊",
            "Haha, totally",
            "Let me know if you need anything!",
        ],
    ),
    (
        Kelly,
        &["OMG 😱😱😱", "wait WHAT", "I literally cannot right now 💅🏽"],
    ),
    (
        Oscar,
        &[
            "Actually, that's not quite right.",
            "Fair enough.",
            "Gracias.",
        ],
    ),
    (
        Creed,
        &["I don't know who this is.", "Nobody steals from Creed."],
    ),
    (
        Ryan,
        &["Let's take this offline.", "Have you heard of my app?"],
    ),
    (Andy, &["Nard Dog approves! 🎶", "Rit dit dit di doo 🎵"]),
    (Kevin, &["ok", "nice", "lol"]),
    (Angela, &["No.", "That's inappropriate."]),
    (Stanley, &["Mm.", "No.", "Is it 5 yet?"]),
    (Tanaka, &["承知いたしました。", "Thank you very much. 🙏"]),
    (Mariam, &["تمام، شكراً!", "Perfect, thank you!"]),
    (KellysMom, &["बहुत अच्छा, बेटा! 😊", "Take care, beta!"]),
    (Phyllis, &["Oh, that's nice dear.", "Bob says hi!"]),
    (Toby, &["Oh. Okay.", "Thanks, I guess."]),
    (Meredith, &["Sure, whatever.", "Is there an open bar?"]),
    // The temp: lines "sent from the phone" outside a scene.
    (
        Me,
        &[
            "On it 👍",
            "Sure, no problem!",
            "Haha",
            "Will do",
            "Thanks!",
        ],
    ),
];

/// Answers from people with no lines of their own (quiet contacts).
pub(super) const STRANGER_REPLIES: &[&str] = &["Who is this?", "Got it, thanks.", "👍"];

/// Each character's favourite reactions. Stanley doesn't react.
pub(super) const REACTS: &[(Who, &[&str])] = &[
    (Michael, &["❤️", "😂", "🎉"]),
    (Dwight, &["👎", "😮"]),
    (Jim, &["😂", "👍"]),
    (Pam, &["❤️", "😊", "🫶🏽"]),
    (Kelly, &["😍", "😱", "💅🏽", "💁🏽‍♀️"]),
    (Oscar, &["👍", "🤓"]),
    (Creed, &["🙏"]),
    (Ryan, &["🔥", "💯"]),
    (Andy, &["🎶", "👍"]),
    (Kevin, &["😂", "🍪"]),
    (Angela, &["😾", "👎"]),
    (Stanley, &[]),
    (Tanaka, &["🙏", "👍"]),
    (Mariam, &["👍", "🙏"]),
    (KellysMom, &["❤️", "🙏"]),
    (Phyllis, &["❤️"]),
    (Toby, &["👍"]),
    (Meredith, &["😂"]),
    (Me, &["👍", "😂"]),
];

/// `who`'s reply pool ([`STRANGER_REPLIES`] for people without one).
pub(super) fn replies(who: Option<Who>) -> &'static [&'static str] {
    who.and_then(|who| REPLIES.iter().find(|(w, _)| *w == who))
        .map_or(STRANGER_REPLIES, |(_, lines)| lines)
}

/// `who`'s favourite reactions (none for people without any).
pub(super) fn reacts(who: Option<Who>) -> &'static [&'static str] {
    who.and_then(|who| REACTS.iter().find(|(w, _)| *w == who))
        .map_or(&[], |(_, emoji)| emoji)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::super::assets::{ALL, attachment_bytes};
    use super::super::cast::PEOPLE;
    use super::super::world::World;
    use super::*;

    fn says_the_trademark(text: &str) -> bool {
        text.to_lowercase().contains("dunder")
    }

    /// Speakers are members of the chat, `reply_to` points backwards, and every
    /// asset has bytes.
    #[test]
    fn scenes_are_consistent() {
        let w = World::build();
        let mut titles = HashSet::new();
        for scene in SCENES {
            let title = scene.title;
            assert!(titles.insert(title), "{title}: titles are unique");
            let conversation = w.conversation(scene.chat).expect("chat exists");
            assert!(w.active.contains(&conversation), "{title}: active chat");
            let members: Vec<_> = w
                .others_in(conversation)
                .into_iter()
                .chain([w.me()])
                .collect();
            let member = |who: Who| w.id_of(who).is_some_and(|id| members.contains(&id));
            assert!(
                (4..=10).contains(&scene.lines.len()),
                "{title}: 4 to 10 lines"
            );
            for (i, line) in scene.lines.iter().enumerate() {
                let at = format!("{title}, line {i}");
                assert!(member(line.who), "{at}: the speaker is in the chat");
                assert!(
                    !line.text.is_empty() || line.attach.is_some(),
                    "{at}: says something"
                );
                if let Some(to) = line.reply_to {
                    assert!(usize::from(to) < i, "{at}: replies to an earlier line");
                }
                if let Some(who) = line.mention {
                    assert!(member(who) && who != line.who, "{at}: mentions a member");
                }
                for &(who, emoji) in line.reactions {
                    assert!(
                        member(who) && who != line.who,
                        "{at}: {emoji} from a member"
                    );
                }
                if line.who == Who::Me {
                    assert_eq!(line.then, Then::Stay, "{at}: the temp's lines stay");
                }
                if let Some(asset) = line.attach {
                    assert!(attachment_bytes(asset.spec().id).is_some(), "{at}: bytes");
                }
                let edit = match line.then {
                    Then::EditTo(text) => text,
                    Then::Stay | Then::Delete => "",
                };
                assert!(
                    !says_the_trademark(line.text) && !says_the_trademark(edit),
                    "{at}"
                );
            }
        }
        for &asset in ALL {
            let bytes = attachment_bytes(asset.spec().id).expect("bytes");
            assert!(!says_the_trademark(&String::from_utf8_lossy(&bytes)));
        }
    }

    #[test]
    fn everyone_has_lines_and_favourites() {
        for who in PEOPLE.iter().map(|p| p.who).chain([Who::Me]) {
            assert!(!replies(Some(who)).is_empty(), "{who:?} can answer");
            assert_eq!(reacts(Some(who)).is_empty(), who == Who::Stanley, "{who:?}");
        }
        assert_eq!(replies(None), STRANGER_REPLIES);
        for (_, lines) in REPLIES {
            assert!(lines.iter().all(|l| !says_the_trademark(l)));
        }
    }
}
