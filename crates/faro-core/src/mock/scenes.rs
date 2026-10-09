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
    // --- Michael ---
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
            say(
                Michael,
                "Draft of my roast for Toby's birthday 🔥🔥 do NOT show Toby",
            )
            .delete(),
            say(Michael, "Wrong chat. Do not tell Toby."),
        ],
    },
    Scene {
        title: "Michael nominates the temp for a Dundie",
        chat: Chat::Direct(Michael),
        lines: &[
            say(Michael, "Okay. Big news. HUGE news."),
            say(Michael, "The Dundies are back and you are NOMINATED 🏆")
                .with(Asset::DundiesNominees),
            say(Michael, "[sticker] 🏆").with(Asset::Trophy),
            say(Me, "I've only been here a week!"),
            say(
                Michael,
                "And yet you have already made an impression.\nOn me.\nMostly on me.",
            )
            .reply(3),
            say(
                Michael,
                "Prepare a speech. 30 seconds. 45 if it's funny. 🎤",
            )
            .react(&[(Me, "😂")]),
        ],
    },
    // --- Dwight ---
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
        title: "Dwight drafts the temp for the beet harvest",
        chat: Chat::Direct(Dwight),
        lines: &[
            say(
                Dwight,
                "Since you have no plans this weekend (I checked), you will help \
                 with the beet harvest.",
            )
            .with(Asset::BeetField),
            say(Me, "How did you check??"),
            say(Dwight, "Irrelevant.").reply(1),
            say(
                Dwight,
                "Mose picks you up at 0500. Frühstück ist um halb sechs. Pünktlich! 🇩🇪",
            ),
            say(Dwight, "Bring boots. Viele Grüße, Dwight."),
            say(Me, "Can I say no?"),
            say(Dwight, "No.").reply(5).react(&[(Me, "😂")]),
        ],
    },
    // --- Jim ---
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
        title: "Jim tells Dwight the temp is a spy",
        chat: Chat::Direct(Jim),
        lines: &[
            say(
                Jim,
                "So, small thing. Dwight thinks you're a spy from corporate.",
            ),
            say(Me, "Why would he think that??"),
            say(Jim, "No idea."),
            say(Jim, "Okay, I told him."),
            say(
                Jim,
                "Just say \"the eagle has landed\" whenever he walks past. Trust me.",
            ),
            say(Me, "The eagle has landed?").reply(4),
            say(Jim, "Perfect. He's going to lose it.").react(&[(Me, "😂")]),
        ],
    },
    // --- Pam ---
    Scene {
        title: "Pam's art show",
        chat: Chat::Direct(Pam),
        lines: &[
            say(Pam, "Hey! So... I have a little art show on Friday 🎨"),
            say(Pam, "It's the office park. Don't laugh 🙈").with(Asset::OfficePark),
            say(Me, "That's beautiful!").react(&[(Pam, "🫶🏽")]),
            say(
                Pam,
                "Aw, thank you! Michael wants to hang it in his office. He asked if \
                 he could sign it too.",
            ),
            say(Me, "See you Friday!"),
            say(Pam, "Yay! There's cheese and wine 🧀🍷"),
        ],
    },
    // --- Kelly ---
    Scene {
        title: "Kelly's gossip",
        chat: Chat::Direct(Kelly),
        lines: &[
            say(Kelly, "OMG OMG OMG 😱😱😱😱😱😱😱"),
            say(Kelly, "ok so you did NOT hear this from me 🤫🤫"),
            say(
                Kelly,
                "Ryan liked a picture of a girl from his business school 💔💔💔 \
                 from TWO YEARS AGO",
            ),
            say(Me, "Oh no 😬"),
            say(Kelly, "I'm changing my profile name. AGAIN. 💅🏽✨💁🏽‍♀️👑"),
            say(Kelly, "also ये बात किसी को मत बताना!!! 🙊🙊").reply(2),
        ],
    },
    // --- Oscar ---
    Scene {
        title: "Oscar fixes Michael's spreadsheet",
        chat: Chat::Direct(Oscar),
        lines: &[
            say(Oscar, "Did Michael send you the quarterly spreadsheet too?"),
            say(Me, "Yes, why?"),
            say(
                Oscar,
                "He's adding up the dates column. The total is March 4th, 2179.",
            ),
            say(
                Oscar,
                "I fixed it. The formulas now point at teh sales figures.",
            )
            .edit("I fixed it. The formulas now point at the sales figures."),
            say(
                Oscar,
                "Dios mío. Mañana I'll show you how pivot tables work.",
            ),
            say(Me, "My lips are sealed 🤐").react(&[(Oscar, "👍")]),
        ],
    },
    // --- Creed ---
    Scene {
        title: "Creed introduces himself",
        chat: Chat::Direct(Creed),
        lines: &[
            say(Creed, "You're new."),
            say(Me, "Who is this?"),
            say(Creed, "Creed. Quality assurance. I think."),
            say(
                Creed,
                "Want to buy some sunglasses? Don't ask where they're from.",
            ),
            say(Me, "No thanks"),
            say(Creed, "Smart. That's why you'll last.").reply(4),
        ],
    },
    // --- Ryan ---
    Scene {
        title: "Ryan's startup pitch",
        chat: Chat::Direct(Ryan),
        lines: &[
            say(Ryan, "Hey. I used to be the temp, so I get it. Respect 👊"),
            say(
                Ryan,
                "Quick pitch. It's like a calendar, but for vibes. \
                 https://example.com/ryan/vybe",
            )
            .edit(
                "Quick pitch. It's like a calendar, but for vibes. \
                 https://vybe.example.com 🚀 (we rebranded)",
            ),
            say(Me, "What does it do?"),
            say(Ryan, "We're pre-product. Which is honestly a strength.").reply(2),
            say(
                Ryan,
                "Deck is here if you're interested: https://example.com/decks/vybe-seed.pdf",
            ),
            say(Me, "I'll take a look"),
        ],
    },
    // --- Andy ---
    Scene {
        title: "Andy's a cappella audition",
        chat: Chat::Direct(Andy),
        lines: &[
            say(Andy, "Hey temp! Big question: tenor or baritone? 🎶"),
            say(Me, "Neither? I don't sing"),
            say(Andy, "Everybody sings. Here's me at Cornell, senior year.").with(Asset::AndyVoice),
            say(Andy, "All four parts. Layered. On a flip phone. 🎤"),
            say(Me, "Impressive!").react(&[(Andy, "🎶")]),
            say(Andy, "Nard Dog thanks you 🐶🎶"),
        ],
    },
    // --- Kevin ---
    Scene {
        title: "Kevin's chili",
        chat: Chat::Direct(Kevin),
        lines: &[
            say(Kevin, "i made chili").with(Asset::Chili),
            say(Kevin, "its my moms recipe"),
            say(Me, "That looks amazing"),
            say(Kevin, "bring a bowl tommorow"),
            say(Kevin, "dont tell angela i used the break room crock pot").reply(0),
            say(Me, "Deal 🤝"),
            say(Kevin, "👍"),
        ],
    },
    // --- Angela ---
    Scene {
        title: "Angela's cat",
        chat: Chat::Direct(Angela),
        lines: &[
            say(Angela, "This is Duchess. She is a show cat.").with(Asset::Cat),
            say(Angela, "She does not like strangers. Or Oscar."),
            say(Me, "She's lovely!"),
            say(Angela, "She is.").reply(2),
            say(
                Angela,
                "You may send me a picture of your cat. If it is clean.",
            ),
            say(Me, "I don't have a cat").react(&[(Angela, "😾")]),
            say(Angela, "I see."),
        ],
    },
    // --- Stanley ---
    Scene {
        title: "Stanley's crossword",
        chat: Chat::Direct(Stanley),
        lines: &[
            say(Stanley, "Seven letters. \"A short break from work.\""),
            say(Me, "Respite?"),
            say(Stanley, "Hm."),
            say(Stanley, "Correct."),
            say(Me, "Happy to help!"),
            say(Stanley, "Don't make it a thing."),
        ],
    },
    // --- Tanaka-san (paper mill, Japan) ---
    Scene {
        title: "Tanaka-san confirms a shipment",
        chat: Chat::Direct(Tanaka),
        lines: &[
            say(Tanaka, "お世話になっております。田中製紙の田中です。"),
            say(
                Tanaka,
                "レターサイズのコピー用紙500箱、来週月曜日に発送いたします。📦",
            ),
            say(Tanaka, "500 boxes of letter paper ship next Monday."),
            say(Me, "Thank you, Tanaka-san! Monday is perfect."),
            say(
                Tanaka,
                "追跡番号は発送後にお送りいたします。どうぞよろしくお願いいたします。",
            )
            .react(&[(Me, "🙏")]),
        ],
    },
    Scene {
        title: "Tanaka-san's invoice",
        chat: Chat::Direct(Tanaka),
        lines: &[
            say(
                Tanaka,
                "請求書をお送りいたします。ご確認のほど、よろしくお願いいたします。",
            )
            .with(Asset::Invoice),
            say(Tanaka, "Invoice 4471 attached. Payment terms: net 30."),
            say(Me, "Got it, I'll pass it to accounting."),
            say(Me, "Oscar says the numbers look right 👍"),
            say(
                Tanaka,
                "ありがとうございます！今後ともよろしくお願いいたします。",
            )
            .reply(3),
        ],
    },
    Scene {
        title: "Tanaka-san and the typhoon",
        chat: Chat::Direct(Tanaka),
        lines: &[
            say(
                Tanaka,
                "台風の影響で、出荷が二日ほど遅れる見込みです。大変申し訳ございません。",
            ),
            say(
                Tanaka,
                "I'm sorry: the typhoon will delay the shipment by about two days.",
            ),
            say(Me, "No problem at all. Stay safe!"),
            say(Tanaka, "お気遣いいただき、ありがとうございます。🙇").reply(2),
        ],
    },
    // --- Mariam (client, Cairo) ---
    Scene {
        title: "Mariam's rush order",
        chat: Chat::Direct(Mariam),
        lines: &[
            say(
                Mariam,
                "مرحباً! نحتاج ٢٠٠ رزمة ورق قبل نهاية الشهر. هل هذا ممكن؟",
            ),
            say(
                Mariam,
                "Hi! 200 reams before the end of the month, is that possible?",
            ),
            say(Me, "Yes! I'll check with the warehouse today."),
            say(Mariam, "ممتاز! شكراً جزيلاً 🌟").reply(2),
            say(Me, "Darryl says yes 👍"),
            say(Mariam, "رائع، سأرسل أمر الشراء غداً."),
        ],
    },
    Scene {
        title: "Mariam sends the purchase order",
        chat: Chat::Direct(Mariam),
        lines: &[
            say(Mariam, "صباح الخير! رقم الطلب PO-4471 — شكراً جزيلاً 🙏"),
            say(
                Mariam,
                "The order number is PO-4471. Delivery to our Cairo warehouse, please.",
            ),
            say(Me, "Got it. It ships on the 28th."),
            say(Mariam, "يوم ٢٨ مناسب جداً. أطيب التحيات من القاهرة 🇪🇬").reply(2),
            say(Me, "Greetings from Scranton! 📄").react(&[(Mariam, "😊")]),
        ],
    },
    Scene {
        title: "Mariam asks for samples",
        chat: Chat::Direct(Mariam),
        lines: &[
            say(Mariam, "هل يمكنكم إرسال عينات من الورق المقوى؟"),
            say(
                Mariam,
                "Could you send a few cardstock samples? Ivory and white.",
            ),
            say(Me, "Of course! Pam is putting a pack together."),
            say(Mariam, "شكراً! فريقكم رائع 😊").react(&[(Me, "❤️")]),
        ],
    },
    // --- Kelly's mom ---
    Scene {
        title: "Mrs. Kapoor invites the temp to Diwali",
        chat: Chat::Direct(KellysMom),
        lines: &[
            say(KellysMom, "नमस्ते बेटा! मैं केली की मम्मी हूँ। 😊"),
            say(
                KellysMom,
                "केली ने बताया कि तुमने अभी-अभी ऑफिस जॉइन किया है। सब ठीक है?",
            ),
            say(Me, "Namaste! Everything's great, thank you 🙏"),
            say(KellysMom, "Good! Kelly says you are very nice."),
            say(
                KellysMom,
                "शनिवार को हमारे घर दिवाली की पार्टी है। तुम ज़रूर आना! 🪔",
            ),
            say(Me, "I'd love to come!").react(&[(KellysMom, "❤️")]),
            say(KellysMom, "बहुत बढ़िया! खाना बहुत होगा, खाली पेट आना। 😄"),
        ],
    },
    Scene {
        title: "Mrs. Kapoor's party details",
        chat: Chat::Direct(KellysMom),
        lines: &[
            say(KellysMom, "बेटा, पार्टी सात बजे शुरू होगी।"),
            say(
                KellysMom,
                "The party starts at 7. Bring nothing, just yourself!",
            ),
            say(Me, "Can I bring dessert?"),
            say(KellysMom, "नहीं नहीं, बस खुद आ जाओ! ❤️").reply(2),
        ],
    },
    // --- Scranton Branch ---
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
            say(Dwight, "fire extinguisher duty is yours. Do NOT lose it.").at(Me),
            say(Me, "Where is it?"),
            say(Dwight, "That is what the drill is for.").reply(3),
            say(Stanley, "I'm not moving for a drill.").reply(0),
        ],
    },
    Scene {
        title: "Michael calls a meeting",
        chat: Chat::Group(GroupKey::Branch),
        lines: &[
            say(
                Michael,
                "Everybody, conference room, five minutes. We are going to discuss a \
                 very important matter that affects every single one of us, so I need \
                 everyone there, including the warehouse, including the temp, and \
                 including Toby, although Toby can sit near the door in case he needs \
                 to leave early, which honestly would be fine.",
            )
            .react(&[(Dwight, "👍"), (Toby, "😢")]),
            say(Jim, "Is this about the email you replied-all to?"),
            say(Michael, "This is about teamwork. And also the email.").reply(1),
            say(Stanley, "Is it mandatory?"),
            say(Michael, "Did I stutter?").reply(3),
            say(Meredith, "Is there cake?"),
        ],
    },
    Scene {
        title: "Pretzel day",
        chat: Chat::Group(GroupKey::Branch),
        lines: &[
            say(Stanley, "It's pretzel day.").with(Asset::Pretzel),
            say(Kevin, "pretzle day!!!"),
            say(
                Dwight,
                "Everyone line up single file. I will be supervising toppings.",
            ),
            say(Stanley, "No.").reply(2).react(&[(Jim, "😂")]),
            say(Michael, "get the sweet mix. Trust me 🥨").at(Me),
            say(Me, "Sweet mix it is!"),
        ],
    },
    Scene {
        title: "Kelly's Diwali party",
        chat: Chat::Group(GroupKey::Branch),
        lines: &[
            say(
                Kelly,
                "OMG everyone!!! Diwali party at my parents' on Saturday 🪔✨💃🏽",
            ),
            say(
                Kelly,
                "coming Saturday right?? my mom already invited you 😭💕",
            )
            .at(Me),
            say(Me, "Wouldn't miss it! 🪔"),
            say(
                Michael,
                "Will there be a talent portion? I have prepared a dance.",
            ),
            say(Kelly, "Michael no. बिल्कुल नहीं। 🙅🏽‍♀️")
                .reply(3)
                .react(&[(Jim, "😂")]),
            say(Angela, "Is it formal? I need to know what to wear."),
            say(
                Kelly,
                "Angela, wear something sparkly!!! ✨ मम्मी will love it 💅🏽",
            )
            .reply(5),
        ],
    },
    // --- Party Planning Committee ---
    Scene {
        title: "Angela's party budget",
        chat: Chat::Group(GroupKey::PartyPlanning),
        lines: &[
            say(
                Angela,
                "The budget for Meredith's birthday is forty dollars. Not a cent more.",
            ),
            say(Phyllis, "Angela, that won't even cover the cake."),
            say(Angela, "Then we will have a smaller cake.").reply(1),
            say(
                Kelly,
                "can we at least have a theme??? like Paris at night 🗼✨💫",
            ),
            say(Angela, "No themes.").reply(3).react(&[(Kelly, "😱")]),
            say(Pam, "can you grab streamers on your lunch break? 🙂").at(Me),
            say(Me, "Sure!"),
            say(Angela, "Beige. Only beige.").reply(5),
        ],
    },
    Scene {
        title: "The summer party theme",
        chat: Chat::Group(GroupKey::PartyPlanning),
        lines: &[
            say(
                Kelly,
                "summer party theme: AROUND THE WORLD 🌍✨ 🇮🇳 🇯🇵 🇲🇽 🇮🇹 🇫🇷 🇩🇪",
            ),
            say(Angela, "Absolutely not. That's over budget.").reply(0),
            say(Phyllis, "I could do a pirate cake for the warehouse 🏴‍☠️"),
            say(Pam, "And a rainbow one for Pride! 🏳️‍🌈").react(&[(Kelly, "😍")]),
            say(Angela, "One cake. Beige.").reply(2),
            say(Me, "I can help decorate!").react(&[(Pam, "❤️")]),
        ],
    },
    // --- Finer Things Club ---
    Scene {
        title: "The Finer Things book pick",
        chat: Chat::Group(GroupKey::FinerThings),
        lines: &[
            say(Oscar, "Proposal for next month: Anna Karenina."),
            say(
                Pam,
                "Ooh, or A Room with a View? I haven't read it since college.",
            ),
            say(Toby, "I'd be happy with either. If that's okay."),
            say(Pam, "tie-breaker: Tolstoy or Forster? 🍷").at(Me),
            say(Me, "Forster!").react(&[(Pam, "❤️")]),
            say(
                Oscar,
                "Muy bien. Thursday in the break room, cafe\u{301} au lait provided ☕",
            ),
        ],
    },
];

/// Lines for answering the temp's messages, by sender.
pub(super) const REPLIES: &[(Who, &[&str])] = &[
    (
        Michael,
        &[
            "That's what she said! 😂",
            "Love it. Love YOU. Professionally. ☺️",
            "Let's circle back on that after my nap. I mean my meeting.",
            "You know what they say: you miss 100% of the texts you don't answer.",
            "Can't talk, in a very important meeting (with myself) 🏆",
            "Agreed. Also, can you pick up my dry cleaning?",
        ],
    ),
    (
        Dwight,
        &[
            "Question. Why are you messaging me during work hours?",
            "False.",
            "Noted. Your file has been updated.",
            "Identity theft is not a joke. Verify yourself: what is my middle name?",
            "Bears. Beets. Battlestar Galactica.",
            "Acceptable. Barely. 🇩🇪",
        ],
    ),
    (
        Jim,
        &[
            "Ha. Nice.",
            "😐",
            "I'll allow it.",
            "Bears. Beets. Battlestar Galactica.",
            "Do not let Dwight see this.",
        ],
    ),
    (
        Pam,
        &[
            "Aw, thanks! 😊",
            "Haha, totally",
            "Let me know if you need anything!",
            "Ooh, good idea 🎨",
        ],
    ),
    (
        Kelly,
        &[
            "OMG 😱😱😱",
            "wait WHAT 💀💀",
            "I literally cannot right now 💅🏽",
            "omg yes!!! 💖💖💖",
            "हाँ हाँ, I know!!! 🙄",
        ],
    ),
    (
        Oscar,
        &[
            "Actually, that's not quite right.",
            "Fair enough.",
            "Gracias.",
            "Let me check the numbers first.",
        ],
    ),
    (
        Creed,
        &[
            "I don't know who this is.",
            "Nobody steals from Creed.",
            "Sure. Cash only.",
            "I've worked here since the seventies. Under a different name.",
        ],
    ),
    (
        Ryan,
        &[
            "Let's take this offline.",
            "Have you seen my app? https://vybe.example.com",
            "Interesting. Can I quote you in my pitch deck?",
            "I used to be the temp. Just saying.",
        ],
    ),
    (
        Andy,
        &[
            "Nard Dog approves! 🎶",
            "Rit dit dit di doo 🎵",
            "Did I mention I went to Cornell? 🐻",
            "Sounds good, buddy!",
        ],
    ),
    (Kevin, &["ok", "nice", "lol", "cool cool", "is there food"]),
    (
        Angela,
        &[
            "No.",
            "That's inappropriate.",
            "I'll allow it this once.",
            "Please don't text me after 6.",
        ],
    ),
    (Stanley, &["Mm.", "No.", "Is it 5 yet?", "Fine."]),
    (
        Tanaka,
        &[
            "承知いたしました。ありがとうございます。",
            "Thank you very much. 🙏",
            "確認いたします。少々お待ちください。",
        ],
    ),
    (
        Mariam,
        &[
            "تمام، شكراً!",
            "Perfect, thank you!",
            "سأتحقق وأعود إليك قريباً.",
        ],
    ),
    (
        KellysMom,
        &[
            "बहुत अच्छा, बेटा! 😊",
            "Take care, beta!",
            "अपना ध्यान रखना! 🙏",
        ],
    ),
    (
        Phyllis,
        &[
            "Oh, that's nice.",
            "Bob says hi!",
            "I'm knitting you a scarf 🧶",
        ],
    ),
    (
        Toby,
        &[
            "Oh. Okay.",
            "Thanks, I guess.",
            "My door is always open. Literally, it's broken.",
        ],
    ),
    (
        Meredith,
        &["Sure, whatever.", "Is there an open bar?", "lol same"],
    ),
    // The temp: lines "sent from the phone" outside a scene.
    (
        Me,
        &[
            "On it 👍",
            "Sure, no problem!",
            "Will do",
            "Thanks!",
            "Sounds good 🙂",
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
