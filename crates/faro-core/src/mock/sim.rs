//! The mock's brain: a seeded, synchronous event scheduler with no I/O.
//!
//! Time is "milliseconds since the mock started" ([`Millis`]). The async driver in
//! `mock.rs` sleeps until [`Sim::next_due`] and forwards what [`Sim::pop_due`]
//! returns. Follow-ups are scheduled relative to each entry's *due* time, never the
//! wall clock, so for a given seed (and the same commands at the same times) the
//! event sequence is identical even if the driver wakes up late.
//!
//! What people say comes from the cast's scenes (`scenes.rs`): the backlog is a
//! dozen or so scenes from the past three days, and each scripted "message"
//! activity plays the next line of a running scene (or starts the next scene in
//! a seeded order, reshuffled once all have played). The other activities are
//! voiced by the cast too: replies and reactions come from each character's reply
//! pool and favourite emoji, "sent from the phone" lines are the temp's, and the
//! profile that changes is Kelly's.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, VecDeque};
use std::time::Duration;

use super::cast::{Chat, KELLY_PROFILES, Who};
use super::scenes::{self, Line, SCENES, Scene, Then};
use super::world::World;
use super::{MockConfig, MockEpoch};
use crate::backend::{BackendError, BackendEvent, OutgoingMessage, Result};
use crate::domain::{
    AttachmentRef, ConnectionState, ConversationId, DeliveryStatus, Direction, Mention, Message,
    MessageBody, MessageRef, Quote, ServiceId, Timestamp, TypingState,
};

/// Milliseconds since the mock started.
pub(super) type Millis = u64;

/// Simulated time from "connecting" to "connected".
const CONNECT_DELAY: Millis = 400;
/// Backlog messages are spread over this window before the epoch.
const BACKLOG_SPAN: Millis = 3 * 24 * 60 * 60 * 1000;
/// The newest backlog scene starts at least this long before the epoch.
const BACKLOG_NEWEST: Millis = 2 * 60 * 60 * 1000;
/// Gap between two lines of a backlog scene.
const BACKLOG_LINE_GAP: std::ops::RangeInclusive<Millis> = 15_000..=240_000;
/// Messages remembered as targets for reactions, replies, edits and deletes.
const RECENT_CAP: usize = 64;
/// Message refs remembered to keep `(sender, sent_at)` unique.
const ISSUED_CAP: usize = 256;
/// Scenes running at once, each in its own chat.
const MAX_RUNNING: usize = 2;
/// How often an answer in a group mentions the temp.
const MENTION_PERCENT: u32 = 25;
/// Scenes started this recently go to the back of the next shuffled order.
const RECENT_SCENES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Activity {
    Message,
    Reply,
    Reaction,
    SyncMessage,
    ReadElsewhere,
    Edit,
    Delete,
    ProfileChange,
    Disconnect,
    DecryptError,
}

/// Relative frequency of each scripted activity.
const ACTIVITY_WEIGHTS: [(Activity, u32); 10] = [
    (Activity::Message, 40),
    (Activity::Reply, 10),
    (Activity::Reaction, 15),
    (Activity::SyncMessage, 8),
    (Activity::ReadElsewhere, 3),
    (Activity::Edit, 4),
    (Activity::Delete, 2),
    (Activity::ProfileChange, 2),
    (Activity::Disconnect, 1),
    (Activity::DecryptError, 1),
];

enum Item {
    /// Run one scripted activity and schedule the next tick.
    Tick,
    /// Boxed: events are large and most queue entries are small.
    Emit(Box<BackendEvent>),
    Connection(ConnectionState),
}

struct Entry {
    due: Millis,
    /// Insertion order: entries due at the same time come out first in, first out.
    seq: u64,
    item: Item,
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.due, self.seq).cmp(&(other.due, other.seq))
    }
}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Entry {}

/// A delivered message the script may react to, quote, edit or delete.
struct Recent {
    conversation: ConversationId,
    target: MessageRef,
    body: MessageBody,
    mentions: Vec<Mention>,
    /// Edited already: generic edits leave it alone.
    edited: bool,
}

/// A scene part-way through.
struct Running {
    /// Index into [`SCENES`].
    scene: usize,
    /// The next line to play.
    next: usize,
    /// What each line played so far can be quoted as (`None` if it was skipped).
    quotes: Vec<Option<Quote>>,
    /// The next line is sent no earlier than this, so lines keep their order.
    ready_at: Millis,
}

/// What a message says, before it has a sender and a time.
#[derive(Default)]
struct Content {
    body: Option<MessageBody>,
    mentions: Vec<Mention>,
    quote: Option<Quote>,
    attachments: Vec<AttachmentRef>,
}

impl Content {
    fn message(
        self,
        conversation: ConversationId,
        sender: ServiceId,
        sent_at: Timestamp,
        direction: Direction,
    ) -> Message {
        Message {
            conversation,
            sender,
            sent_at,
            direction,
            body: self.body,
            mentions: self.mentions,
            quote: self.quote,
            attachments: self.attachments,
            reactions: vec![],
            edited_at: None,
            deleted: false,
        }
    }
}

pub(super) struct Sim {
    rng: fastrand::Rng,
    world: World,
    /// Wall-clock milliseconds at `Millis` 0.
    epoch_ms: u64,
    /// Mean time between scripted activities; `None` = no scripted activity.
    activity: Option<Millis>,
    disconnects: bool,
    errors: bool,
    queue: BinaryHeap<Reverse<Entry>>,
    next_seq: u64,
    connection: ConnectionState,
    /// No new outage starts before this time (avoids overlapping outages).
    outage_ends: Millis,
    /// Events produced while offline, delivered on reconnect (like Signal's queue).
    held: Vec<BackendEvent>,
    recent: VecDeque<Recent>,
    issued: VecDeque<MessageRef>,
    /// Scene indices in playing order; reshuffled when `cursor` reaches the end.
    order: Vec<usize>,
    cursor: usize,
    /// The last [`RECENT_SCENES`] scenes started, oldest first.
    started: VecDeque<usize>,
    running: Vec<Running>,
    /// Answers from reply pools (generic deletes only take back these).
    answers: VecDeque<MessageRef>,
    /// Index into [`KELLY_PROFILES`] of Kelly's current profile name.
    kelly_profile: usize,
}

impl Sim {
    pub(super) fn new(config: &MockConfig, world: World) -> Self {
        let epoch_ms = match config.epoch {
            MockEpoch::Fixed(ts) => ts.as_millis(),
            MockEpoch::Now => wall_clock_ms(),
        };
        let mut rng = fastrand::Rng::with_seed(config.seed);
        let mut order: Vec<usize> = (0..SCENES.len()).collect();
        rng.shuffle(&mut order);
        let mut sim = Self {
            rng,
            world,
            epoch_ms,
            activity: config.activity.map(|d| millis(d).max(1)),
            disconnects: config.disconnects,
            errors: config.errors,
            queue: BinaryHeap::new(),
            next_seq: 0,
            connection: ConnectionState::Disconnected,
            outage_ends: CONNECT_DELAY,
            held: Vec::new(),
            recent: VecDeque::new(),
            issued: VecDeque::new(),
            order,
            cursor: 0,
            started: VecDeque::new(),
            running: Vec::new(),
            answers: VecDeque::new(),
            kelly_profile: 0,
        };
        sim.schedule(0, Item::Connection(ConnectionState::Connecting));
        sim.schedule(CONNECT_DELAY, Item::Connection(ConnectionState::Connected));
        sim.schedule_backlog(config.backlog);
        if let Some(mean) = sim.activity {
            let first = CONNECT_DELAY + sim.jitter(mean);
            sim.schedule(first, Item::Tick);
        }
        sim
    }

    pub(super) fn world(&self) -> &World {
        &self.world
    }

    pub(super) fn wall_clock_at(&self, at: Millis) -> Timestamp {
        Timestamp::from_millis(self.epoch_ms.saturating_add(at))
    }

    pub(super) fn next_due(&self) -> Option<Millis> {
        self.queue.peek().map(|Reverse(e)| e.due)
    }

    /// Process everything due at or before `now`, in order, and return the events.
    pub(super) fn pop_due(&mut self, now: Millis) -> Vec<BackendEvent> {
        let mut out = Vec::new();
        while self.next_due().is_some_and(|due| due <= now) {
            let Some(Reverse(entry)) = self.queue.pop() else {
                break;
            };
            match entry.item {
                Item::Tick => self.tick(entry.due),
                Item::Emit(event) => {
                    if self.is_online() {
                        self.deliver(*event, &mut out);
                    } else {
                        self.held.push(*event);
                    }
                }
                Item::Connection(state) => {
                    self.connection = state;
                    out.push(BackendEvent::Connection(state));
                    if self.is_online() {
                        for event in std::mem::take(&mut self.held) {
                            self.deliver(event, &mut out);
                        }
                    }
                }
            }
        }
        out
    }

    pub(super) fn is_online(&self) -> bool {
        self.connection == ConnectionState::Connected
    }

    /// Fails like a real backend would while the connection is down.
    pub(super) fn require_online(&self) -> Result<()> {
        if self.is_online() {
            Ok(())
        } else {
            Err(BackendError::Network("mock backend is offline".into()))
        }
    }

    /// Simulated network latency for an outgoing request.
    pub(super) fn request_latency(&mut self) -> Duration {
        Duration::from_millis(self.rng.u64(50..=250))
    }

    /// The message reached the "server": report progress and maybe get an answer.
    pub(super) fn sent(
        &mut self,
        now: Millis,
        to: ConversationId,
        msg: OutgoingMessage,
    ) -> Result<()> {
        self.require_online()?;
        if let ConversationId::Group(id) = to
            && !self.world.has_group(id)
        {
            return Err(BackendError::NotFound);
        }
        let target = MessageRef {
            sender: self.world.me(),
            sent_at: msg.sent_at,
        };
        self.remember(to, target, msg.body, msg.mentions);
        let status = DeliveryStatus::Sent;
        self.emit(
            now,
            BackendEvent::DeliveryUpdate {
                message: target,
                status,
                by: None,
                updated_at: None,
            },
        );
        let read_at = self.schedule_receipts(now, to, target);

        if self.chance(60)
            && let Some(sender) = self.answerer(to)
        {
            let quote = self.chance(30).then(|| self.quote_of(target)).flatten();
            let start = read_at + self.rng.u64(1_000..=3_000);
            self.schedule_answer(start, to, sender, quote);
        }
        Ok(())
    }

    /// Edits and deletes only apply to this account's own messages.
    pub(super) fn require_own(&self, target: MessageRef) -> Result<()> {
        if target.sender == self.world.me() {
            Ok(())
        } else {
            Err(BackendError::NotFound)
        }
    }

    /// Start an outage at `now` lasting `duration`, unless one is already running.
    pub(super) fn schedule_outage(&mut self, now: Millis, duration: Duration) {
        if now < self.outage_ends {
            return;
        }
        let back = now.saturating_add(millis(duration));
        self.outage_ends = back + CONNECT_DELAY;
        self.schedule(now, Item::Connection(ConnectionState::Disconnected));
        self.schedule(back, Item::Connection(ConnectionState::Connecting));
        self.schedule(
            back + CONNECT_DELAY,
            Item::Connection(ConnectionState::Connected),
        );
    }

    pub(super) fn schedule_error(&mut self, now: Millis, error: BackendError) {
        self.emit(now, BackendEvent::Error(error));
    }

    // --- scheduling helpers ---

    fn schedule(&mut self, due: Millis, item: Item) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.queue.push(Reverse(Entry { due, seq, item }));
    }

    fn emit(&mut self, due: Millis, event: BackendEvent) {
        self.schedule(due, Item::Emit(Box::new(event)));
    }

    /// Delivered, then read. Returns when "read" happens.
    fn schedule_receipts(
        &mut self,
        now: Millis,
        to: ConversationId,
        message: MessageRef,
    ) -> Millis {
        let delivered_at = now + self.rng.u64(300..=2_000);
        let read_at = delivered_at + self.rng.u64(1_000..=20_000);
        let by = self.others_in(to).first().copied();
        let delivered = DeliveryStatus::Delivered;
        let delivered_ts = self.wall_clock_at(delivered_at);
        self.emit(
            delivered_at,
            BackendEvent::DeliveryUpdate {
                message,
                status: delivered,
                by,
                updated_at: Some(delivered_ts),
            },
        );
        let read = DeliveryStatus::Read;
        let read_ts = self.wall_clock_at(read_at);
        self.emit(
            read_at,
            BackendEvent::DeliveryUpdate {
                message,
                status: read,
                by,
                updated_at: Some(read_ts),
            },
        );
        read_at
    }

    /// Someone starts typing at `start` and sends `content` a little later.
    /// Returns the message and when it is sent.
    fn schedule_incoming(
        &mut self,
        start: Millis,
        conversation: ConversationId,
        sender: ServiceId,
        content: Content,
    ) -> (MessageRef, Millis) {
        let typing = |state| BackendEvent::Typing {
            conversation,
            sender,
            state,
        };
        let typed = start + self.typing_time();
        let (sent_at, sent) = self.stamp(sender, typed);
        let message = content.message(conversation, sender, sent_at, Direction::Incoming);
        let reference = message.reference();
        self.emit(start, typing(TypingState::Started));
        self.emit(sent, BackendEvent::Message(message));
        self.emit(sent, typing(TypingState::Stopped));
        (reference, sent)
    }

    /// Sent by the temp from their phone at `at`: arrives as an outgoing message.
    /// Returns the message and when it is sent.
    fn schedule_outgoing(
        &mut self,
        at: Millis,
        conversation: ConversationId,
        content: Content,
    ) -> (MessageRef, Millis) {
        let me = self.world.me();
        let (sent_at, sent) = self.stamp(me, at);
        let direction = Direction::Outgoing(DeliveryStatus::Sent);
        let message = content.message(conversation, me, sent_at, direction);
        let target = message.reference();
        self.emit(sent, BackendEvent::Message(message));
        self.schedule_receipts(sent, conversation, target);
        (target, sent)
    }

    /// `sender` answers in `conversation` with a line from their reply pool.
    fn schedule_answer(
        &mut self,
        start: Millis,
        conversation: ConversationId,
        sender: ServiceId,
        quote: Option<Quote>,
    ) {
        let who = self.world.who_is(sender);
        let text = self
            .rng
            .choice(scenes::replies(who))
            .copied()
            .unwrap_or("👍");
        let in_group = matches!(conversation, ConversationId::Group(_));
        let mention = (in_group && self.chance(MENTION_PERCENT)).then_some(Who::Me);
        let content = Content {
            quote,
            ..self.content(text, mention)
        };
        let (answer, _) = self.schedule_incoming(start, conversation, sender, content);
        if self.answers.len() >= RECENT_CAP {
            self.answers.pop_front();
        }
        self.answers.push_back(answer);
    }

    /// Messages "queued on the server" while faro was closed: `count` lines from
    /// whole scenes (the last one may stop part-way and carry on live), dated
    /// over the past few days and delivered right after connecting.
    fn schedule_backlog(&mut self, count: usize) {
        let mut plays: Vec<(usize, usize)> = Vec::new();
        let mut left = count;
        while left > 0 {
            let Some(scene) = self.next_scene() else {
                break;
            };
            let len = SCENES.get(scene).map_or(0, |s| s.lines.len());
            if len == 0 {
                break;
            }
            let lines = len.min(left);
            plays.push((scene, lines));
            left -= lines;
        }
        let scenes = u64::try_from(plays.len()).unwrap_or(1).max(1);
        let slot = (BACKLOG_SPAN - BACKLOG_NEWEST) / scenes;
        // (wall-clock ms, event), sorted before delivery.
        let mut events: Vec<(u64, BackendEvent)> = Vec::new();
        let mut age_slot = scenes;
        for (scene_index, lines) in plays {
            age_slot -= 1;
            let Some(scene) = SCENES.get(scene_index) else {
                continue;
            };
            let age = BACKLOG_NEWEST + slot * age_slot + self.rng.u64(0..=slot / 2);
            let mut wall = self.epoch_ms.saturating_sub(age);
            let mut quotes = Vec::with_capacity(lines);
            for (i, line) in scene.lines.iter().take(lines).enumerate() {
                if i > 0 {
                    wall += self.rng.u64(BACKLOG_LINE_GAP);
                }
                let quote = self.backlog_line(scene, line, &quotes, wall, &mut events);
                quotes.push(quote);
            }
            if lines < scene.lines.len() {
                self.running.push(Running {
                    scene: scene_index,
                    next: lines,
                    quotes,
                    ready_at: 0,
                });
            }
        }
        // Stable: a message's follow-ups never overtake it.
        events.sort_by_key(|(at, _)| *at);
        for (_, event) in events {
            self.emit(CONNECT_DELAY, event);
        }
    }

    /// One backlog line sent at `wall` (and its follow-ups) into `events`.
    fn backlog_line(
        &mut self,
        scene: &Scene,
        line: &Line,
        quotes: &[Option<Quote>],
        wall: u64,
        events: &mut Vec<(u64, BackendEvent)>,
    ) -> Option<Quote> {
        let conversation = self.world.conversation(scene.chat)?;
        let sender = self.world.id_of(line.who)?;
        let direction = if line.who == Who::Me {
            Direction::Outgoing(DeliveryStatus::Read)
        } else {
            Direction::Incoming
        };
        let sent_at = self.stamp_wall(sender, wall);
        let content = self.line_content(line, quotes);
        let message = content.message(conversation, sender, sent_at, direction);
        let quote = Quote {
            target: message.reference(),
            preview: message.body.clone(),
        };
        let at = sent_at.as_millis();
        events.push((at, BackendEvent::Message(message)));
        for (delay, event) in self.follow_ups(conversation, line, quote.target, at) {
            events.push((at + delay, event));
        }
        Some(quote)
    }

    // --- scenes ---

    /// The next scene in the seeded order whose chat has no scene running.
    fn next_scene(&mut self) -> Option<usize> {
        if self.cursor >= self.order.len() {
            // Reshuffle, keeping what just played away from the front.
            self.rng.shuffle(&mut self.order);
            let started = &self.started;
            self.order.sort_by_key(|s| started.contains(s));
            self.cursor = 0;
        }
        let busy: Vec<Chat> = self
            .running
            .iter()
            .filter_map(|r| SCENES.get(r.scene))
            .map(|s| s.chat)
            .collect();
        let offset = self.order.get(self.cursor..)?.iter().position(|&s| {
            SCENES
                .get(s)
                .is_some_and(|scene| !busy.contains(&scene.chat))
        })?;
        self.order.swap(self.cursor, self.cursor + offset);
        let scene = self.order.get(self.cursor).copied()?;
        self.cursor += 1;
        if self.started.len() >= RECENT_SCENES {
            self.started.pop_front();
        }
        self.started.push_back(scene);
        Some(scene)
    }

    /// Which running scene the next line comes from, starting one if needed.
    fn pick_running(&mut self, now: Millis) -> Option<usize> {
        let ready: Vec<usize> = (0..self.running.len())
            .filter(|&i| self.running.get(i).is_some_and(|r| r.ready_at <= now))
            .collect();
        let start_new = self.running.is_empty()
            || (self.running.len() < MAX_RUNNING && (ready.is_empty() || self.chance(35)));
        if start_new && let Some(scene) = self.next_scene() {
            if let Some(s) = SCENES.get(scene) {
                tracing::debug!(scene = s.title, "mock scene starts");
            }
            self.running.push(Running {
                scene,
                next: 0,
                quotes: Vec::new(),
                ready_at: 0,
            });
            return Some(self.running.len() - 1);
        }
        if let Some(slot) = self.rng.choice(ready) {
            return Some(slot);
        }
        // Everyone is mid-sentence: queue behind whoever finishes first.
        (0..self.running.len()).min_by_key(|&i| self.running.get(i).map(|r| r.ready_at))
    }

    /// A running scene whose next line `wanted` accepts, ready to be played.
    fn running_where(&self, now: Millis, wanted: impl Fn(&Line) -> bool) -> Option<usize> {
        self.running.iter().position(|r| {
            r.ready_at <= now
                && SCENES
                    .get(r.scene)
                    .and_then(|s| s.lines.get(r.next))
                    .is_some_and(&wanted)
        })
    }

    /// Play the next line of running scene `slot`. A line that can't be played
    /// is skipped, so a scene never gets stuck.
    fn play_next(&mut self, now: Millis, slot: usize) -> Option<()> {
        let running = self.running.get(slot)?;
        let scene = SCENES.get(running.scene)?;
        let index = running.next;
        let start = now.max(running.ready_at);
        let played = scene
            .lines
            .get(index)
            .and_then(|line| self.play_line(start, scene, line, slot));
        if let Some(running) = self.running.get_mut(slot) {
            running.next = index + 1;
            running
                .quotes
                .push(played.as_ref().map(|(quote, _)| quote.clone()));
            if let Some((_, sent)) = played {
                running.ready_at = sent + 1;
            }
        }
        if index + 1 >= scene.lines.len() {
            self.running.remove(slot);
        }
        played.map(|_| ())
    }

    /// Schedule `line` from `start`; returns how it can be quoted and when it's sent.
    fn play_line(
        &mut self,
        start: Millis,
        scene: &Scene,
        line: &Line,
        slot: usize,
    ) -> Option<(Quote, Millis)> {
        let conversation = self.world.conversation(scene.chat)?;
        let sender = self.world.id_of(line.who)?;
        let quotes = self.running.get(slot).map(|r| r.quotes.clone())?;
        let content = self.line_content(line, &quotes);
        let preview = content.body.clone();
        let (target, sent) = if line.who == Who::Me {
            self.schedule_outgoing(start, conversation, content)
        } else {
            self.schedule_incoming(start, conversation, sender, content)
        };
        let wall = self.epoch_ms.saturating_add(sent);
        for (delay, event) in self.follow_ups(conversation, line, target, wall) {
            self.emit(sent + delay, event);
        }
        Some((Quote { target, preview }, sent))
    }

    /// What `line` says; `quotes` are the scene's lines played so far.
    fn line_content(&self, line: &Line, quotes: &[Option<Quote>]) -> Content {
        let quote = line
            .reply_to
            .and_then(|i| quotes.get(usize::from(i)).cloned().flatten());
        let attachments = line
            .attach
            .and_then(super::assets::Asset::attachment)
            .into_iter()
            .collect();
        Content {
            quote,
            attachments,
            ..self.content(line.text, line.mention)
        }
    }

    /// Body and mentions for `text`, with `mention` (if any) in front.
    fn content(&self, text: &str, mention: Option<Who>) -> Content {
        match mention.and_then(|who| self.world.id_of(who)) {
            Some(who) => {
                let body = if text.is_empty() {
                    "\u{FFFC}".to_owned()
                } else {
                    format!("\u{FFFC} {text}")
                };
                Content {
                    body: Some(MessageBody::new(body)),
                    mentions: vec![Mention {
                        who,
                        start: 0,
                        len: '\u{FFFC}'.len_utf8(),
                    }],
                    ..Content::default()
                }
            }
            None if text.is_empty() => Content::default(),
            None => Content {
                body: Some(MessageBody::new(text)),
                ..Content::default()
            },
        }
    }

    /// The reactions, edit and delete written into `line`, sent at wall-clock
    /// `wall`, as `(delay, event)` pairs.
    fn follow_ups(
        &mut self,
        conversation: ConversationId,
        line: &Line,
        target: MessageRef,
        wall: u64,
    ) -> Vec<(Millis, BackendEvent)> {
        let mut out = Vec::new();
        for &(who, emoji) in line.reactions {
            if let Some(sender) = self.world.id_of(who) {
                let reaction = BackendEvent::Reaction {
                    conversation,
                    target,
                    sender,
                    emoji: Some(emoji.to_owned()),
                };
                out.push((self.later(), reaction));
            }
        }
        match line.then {
            Then::Stay => {}
            Then::EditTo(text) => {
                let delay = self.later();
                let content = self.content(text, line.mention);
                if let Some(body) = content.body {
                    let edited = BackendEvent::Edited {
                        conversation,
                        target,
                        body,
                        mentions: content.mentions,
                        edited_at: Timestamp::from_millis(wall + delay),
                    };
                    out.push((delay, edited));
                }
            }
            Then::Delete => {
                let deleted = BackendEvent::Deleted {
                    conversation,
                    target,
                };
                out.push((self.later(), deleted));
            }
        }
        out
    }

    /// Conversations with a scene running: generic activity keeps out of them.
    fn busy(&self) -> Vec<ConversationId> {
        self.running
            .iter()
            .filter_map(|r| SCENES.get(r.scene))
            .filter_map(|s| self.world.conversation(s.chat))
            .collect()
    }

    // --- scripted activity ---

    fn tick(&mut self, now: Millis) {
        if let Some(mean) = self.activity {
            let next = now + self.jitter(mean);
            self.schedule(next, Item::Tick);
        }
        if !self.is_online() {
            return;
        }
        let options: Vec<(Activity, u32)> = ACTIVITY_WEIGHTS
            .into_iter()
            .filter(|(a, _)| match a {
                Activity::Disconnect => self.disconnects,
                Activity::DecryptError => self.errors,
                Activity::Message
                | Activity::Reply
                | Activity::Reaction
                | Activity::SyncMessage
                | Activity::ReadElsewhere
                | Activity::Edit
                | Activity::Delete
                | Activity::ProfileChange => true,
            })
            .collect();
        let activity = weighted(&mut self.rng, &options).unwrap_or(Activity::Message);
        if !self.run(activity, now) {
            // No suitable target yet (e.g. nothing to react to): just chat.
            self.run(Activity::Message, now);
        }
    }

    /// Returns `false` if the activity had nothing to act on.
    fn run(&mut self, activity: Activity, now: Millis) -> bool {
        match activity {
            Activity::Message => self.chat(now),
            Activity::Reply => self.reply(now),
            Activity::Reaction => self.react(now),
            Activity::SyncMessage => self.sync_message(now),
            Activity::ReadElsewhere => self.read_elsewhere(now),
            Activity::Edit => self.edit(now),
            Activity::Delete => self.delete(now),
            Activity::ProfileChange => self.profile_change(now),
            Activity::Disconnect => {
                let duration = Duration::from_millis(self.rng.u64(2_000..=10_000));
                self.schedule_outage(now, duration);
                Some(())
            }
            Activity::DecryptError => self.decrypt_error(now),
        }
        .is_some()
    }

    /// The next line of a scene.
    fn chat(&mut self, now: Millis) -> Option<()> {
        let slot = self.pick_running(now)?;
        self.play_next(now, slot)
    }

    /// A scripted reply if one is next; otherwise someone quotes a recent
    /// message and answers it from their reply pool.
    fn reply(&mut self, now: Millis) -> Option<()> {
        if let Some(slot) = self.running_where(now, |l| l.reply_to.is_some()) {
            return self.play_next(now, slot);
        }
        let busy = self.busy();
        let (conversation, target) = self.pick_recent(|r| !busy.contains(&r.conversation))?;
        let mut senders = self.others_in(conversation);
        senders.retain(|s| *s != target.sender);
        let sender = self.rng.choice(senders)?;
        let quote = self.quote_of(target);
        self.schedule_answer(now, conversation, sender, quote);
        Some(())
    }

    /// Someone reacts to a recent message with one of their favourite emoji.
    fn react(&mut self, now: Millis) -> Option<()> {
        let (conversation, target) = self.pick_recent(|_| true)?;
        let reactors: Vec<ServiceId> = self
            .others_in(conversation)
            .into_iter()
            .filter(|s| *s != target.sender)
            .filter(|s| !scenes::reacts(self.world.who_is(*s)).is_empty())
            .collect();
        let sender = self.rng.choice(reactors)?;
        let favourites = scenes::reacts(self.world.who_is(sender));
        let emoji = self.rng.choice(favourites).map(|e| (*e).to_owned());
        let reaction = BackendEvent::Reaction {
            conversation,
            target,
            sender,
            emoji,
        };
        self.emit(now, reaction);
        Some(())
    }

    /// Sent by the temp from their phone: the scene's line if it's theirs,
    /// otherwise a short answer in a quiet chat where someone else spoke last.
    fn sync_message(&mut self, now: Millis) -> Option<()> {
        if let Some(slot) = self.running_where(now, |l| l.who == Who::Me) {
            return self.play_next(now, slot);
        }
        let me = self.world.me();
        let mut seen = self.busy();
        let mut conversation = None;
        for r in self.recent.iter().rev() {
            if seen.contains(&r.conversation) {
                continue;
            }
            if r.target.sender != me {
                conversation = Some(r.conversation);
                break;
            }
            // The temp spoke last here: nothing to answer.
            seen.push(r.conversation);
        }
        let conversation = conversation?;
        let text = self
            .rng
            .choice(scenes::replies(Some(Who::Me)))
            .copied()
            .unwrap_or("👍");
        let content = self.content(text, None);
        self.schedule_outgoing(now, conversation, content);
        Some(())
    }

    /// The owner read a conversation on their phone: a read sync for its
    /// newest incoming message (D70).
    fn read_elsewhere(&mut self, now: Millis) -> Option<()> {
        let me = self.world.me();
        let (conversation, _) = self.pick_recent(|r| r.target.sender != me)?;
        let newest = self
            .recent
            .iter()
            .rev()
            .find(|r| r.conversation == conversation && r.target.sender != me)?
            .target;
        self.emit(now, BackendEvent::ReadElsewhere(vec![newest]));
        Some(())
    }

    /// Someone adds one of their favourite emoji to a message they sent.
    fn edit(&mut self, now: Millis) -> Option<()> {
        let me = self.world.me();
        let busy = self.busy();
        let candidates: Vec<usize> = (0..self.recent.len())
            .filter(|&i| {
                self.recent.get(i).is_some_and(|r| {
                    r.target.sender != me
                        && !r.edited
                        && !busy.contains(&r.conversation)
                        && !scenes::reacts(self.world.who_is(r.target.sender)).is_empty()
                })
            })
            .collect();
        let recent = self.recent.get(self.rng.choice(candidates)?)?;
        let (conversation, target) = (recent.conversation, recent.target);
        let (old, mentions) = (recent.body.expose().to_owned(), recent.mentions.clone());
        let favourites = scenes::reacts(self.world.who_is(target.sender));
        let emoji = self.rng.choice(favourites)?;
        let edited = BackendEvent::Edited {
            conversation,
            target,
            body: MessageBody::new(format!("{old} {emoji}")),
            mentions,
            edited_at: self.wall_clock_at(now),
        };
        self.emit(now, edited);
        Some(())
    }

    /// Michael takes back one of his answers (written deletes come with their
    /// scene).
    fn delete(&mut self, now: Millis) -> Option<()> {
        let michael = self.world.id_of(Who::Michael)?;
        let answers: Vec<MessageRef> = self.answers.iter().copied().collect();
        let (conversation, target) =
            self.pick_recent(|r| r.target.sender == michael && answers.contains(&r.target))?;
        let deleted = BackendEvent::Deleted {
            conversation,
            target,
        };
        self.emit(now, deleted);
        Some(())
    }

    /// Kelly changes her profile name. Again.
    fn profile_change(&mut self, now: Millis) -> Option<()> {
        let id = self.world.id_of(Who::Kelly)?;
        self.kelly_profile = (self.kelly_profile + 1) % KELLY_PROFILES.len().max(1);
        let name = KELLY_PROFILES.get(self.kelly_profile)?;
        let contact = self
            .world
            .contacts
            .iter_mut()
            .find(|c| c.service_id == id)?;
        contact.profile_name = Some((*name).to_owned());
        let updated = BackendEvent::ContactUpdated(contact.clone());
        self.emit(now, updated);
        Some(())
    }

    fn decrypt_error(&mut self, now: Millis) -> Option<()> {
        let conversation = self.rng.choice(self.world.active.clone())?;
        let sender = self.rng.choice(self.others_in(conversation))?;
        // Service IDs only: error details must never contain bodies or phones.
        let detail = format!("mock: message from {sender} failed to decrypt");
        self.emit(now, BackendEvent::Error(BackendError::Protocol(detail)));
        Some(())
    }

    // --- state ---

    /// Push an event out and keep the script's view of recent messages current.
    fn deliver(&mut self, event: BackendEvent, out: &mut Vec<BackendEvent>) {
        match &event {
            BackendEvent::Message(m) => {
                if let Some(body) = &m.body {
                    self.remember(
                        m.conversation,
                        m.reference(),
                        body.clone(),
                        m.mentions.clone(),
                    );
                }
            }
            BackendEvent::Edited {
                target,
                body,
                mentions,
                ..
            } => {
                if let Some(r) = self.recent.iter_mut().find(|r| r.target == *target) {
                    r.body = body.clone();
                    r.mentions.clone_from(mentions);
                    r.edited = true;
                }
            }
            BackendEvent::Deleted { target, .. } => self.recent.retain(|r| r.target != *target),
            BackendEvent::Connection(_)
            | BackendEvent::DeliveryUpdate { .. }
            | BackendEvent::Reaction { .. }
            | BackendEvent::Typing { .. }
            | BackendEvent::ReadElsewhere(_)
            | BackendEvent::ContactUpdated(_)
            | BackendEvent::GroupUpdated(_)
            | BackendEvent::Error(_) => {}
        }
        out.push(event);
    }

    fn remember(
        &mut self,
        conversation: ConversationId,
        target: MessageRef,
        body: MessageBody,
        mentions: Vec<Mention>,
    ) {
        if self.recent.len() >= RECENT_CAP {
            self.recent.pop_front();
        }
        self.recent.push_back(Recent {
            conversation,
            target,
            body,
            mentions,
            edited: false,
        });
    }

    fn pick_recent(
        &mut self,
        filter: impl Fn(&Recent) -> bool,
    ) -> Option<(ConversationId, MessageRef)> {
        let candidates: Vec<(ConversationId, MessageRef)> = self
            .recent
            .iter()
            .filter(|r| filter(r))
            .map(|r| (r.conversation, r.target))
            .collect();
        self.rng.choice(candidates)
    }

    fn quote_of(&self, target: MessageRef) -> Option<Quote> {
        let recent = self.recent.iter().find(|r| r.target == target)?;
        Some(Quote {
            target,
            preview: Some(recent.body.clone()),
        })
    }

    /// Everyone but this account who can write in `conversation`.
    fn others_in(&self, conversation: ConversationId) -> Vec<ServiceId> {
        let me = self.world.me();
        let mut others = self.world.others_in(conversation);
        others.retain(|s| *s != me);
        others
    }

    /// Who answers the temp in `conversation`: in groups, someone from the cast
    /// if there is anyone.
    fn answerer(&mut self, conversation: ConversationId) -> Option<ServiceId> {
        let others = self.others_in(conversation);
        let cast: Vec<ServiceId> = others
            .iter()
            .copied()
            .filter(|s| self.world.who_is(*s).is_some())
            .collect();
        if cast.is_empty() {
            self.rng.choice(others)
        } else {
            self.rng.choice(cast)
        }
    }

    /// A unique `sent_at` for `sender` at simulated time `at` or just after,
    /// and the simulated time it stands for: send the message then, so it is
    /// never dated after it arrives.
    fn stamp(&mut self, sender: ServiceId, at: Millis) -> (Timestamp, Millis) {
        let sent_at = self.stamp_wall(sender, self.epoch_ms.saturating_add(at));
        let sent = sent_at.as_millis().saturating_sub(self.epoch_ms);
        (sent_at, sent)
    }

    /// Signal identifies messages by `(sender, sent_at)`, so bump on collision.
    fn stamp_wall(&mut self, sender: ServiceId, mut ms: u64) -> Timestamp {
        while self
            .issued
            .iter()
            .any(|r| r.sender == sender && r.sent_at.as_millis() == ms)
        {
            ms += 1;
        }
        let sent_at = Timestamp::from_millis(ms);
        if self.issued.len() >= ISSUED_CAP {
            self.issued.pop_front();
        }
        self.issued.push_back(MessageRef { sender, sent_at });
        sent_at
    }

    fn chance(&mut self, percent: u32) -> bool {
        self.rng.u32(0..100) < percent
    }

    /// Uniform in `[mean / 2, mean * 3 / 2]`.
    fn jitter(&mut self, mean: Millis) -> Millis {
        let half = mean / 2;
        self.rng.u64(half..=mean + half).max(1)
    }

    /// How long someone types: 1–5 s, shorter in a flood so lines keep up.
    fn typing_time(&mut self) -> Millis {
        let max = self
            .activity
            .map_or(5_000, |mean| mean.saturating_mul(2))
            .clamp(2, 5_000);
        self.rng.u64(max / 5..=max)
    }

    /// When a written reaction, edit or delete follows its message: 1–8 s,
    /// sooner in a flood.
    fn later(&mut self) -> Millis {
        let mean = self.activity.unwrap_or(4_000).min(4_000);
        self.rng.u64(mean / 4..=mean * 2).max(1)
    }
}

fn weighted<T: Copy>(rng: &mut fastrand::Rng, options: &[(T, u32)]) -> Option<T> {
    let total: u32 = options.iter().map(|(_, w)| w).sum();
    if total == 0 {
        return None;
    }
    let mut roll = rng.u32(0..total);
    for &(item, weight) in options {
        if roll < weight {
            return Some(item);
        }
        roll -= weight;
    }
    None
}

pub(super) fn millis(d: Duration) -> Millis {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn wall_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the simulation with no async runtime: jump from due time to due time.
    fn run(sim: &mut Sim, events: usize) -> Vec<BackendEvent> {
        let mut out = Vec::new();
        while out.len() < events {
            let due = sim.next_due().expect("scripted activity never runs out");
            out.extend(sim.pop_due(due));
        }
        out.truncate(events);
        out
    }

    fn sim(seed: u64) -> Sim {
        Sim::new(&MockConfig::deterministic(seed), World::build())
    }

    #[test]
    fn same_seed_same_events() {
        assert_eq!(run(&mut sim(7), 500), run(&mut sim(7), 500));
        assert_ne!(run(&mut sim(7), 500), run(&mut sim(8), 500));
    }

    #[test]
    fn script_exercises_every_event_kind() {
        let events = run(&mut sim(1), 3_000);
        let has = |f: fn(&BackendEvent) -> bool| events.iter().any(f);
        assert!(has(
            |e| matches!(e, BackendEvent::Message(m) if m.direction == Direction::Incoming)
        ));
        assert!(has(
            |e| matches!(e, BackendEvent::Message(m) if matches!(m.direction, Direction::Outgoing(_)))
        ));
        assert!(has(
            |e| matches!(e, BackendEvent::Message(m) if m.quote.is_some())
        ));
        assert!(has(
            |e| matches!(e, BackendEvent::Message(m) if !m.mentions.is_empty())
        ));
        assert!(has(
            |e| matches!(e, BackendEvent::Message(m) if !m.attachments.is_empty())
        ));
        assert!(has(|e| matches!(e, BackendEvent::Reaction { .. })));
        assert!(has(|e| matches!(e, BackendEvent::Typing { .. })));
        assert!(has(|e| matches!(e, BackendEvent::ReadElsewhere(_))));
        assert!(has(|e| matches!(e, BackendEvent::Edited { .. })));
        assert!(has(|e| matches!(e, BackendEvent::Deleted { .. })));
        assert!(has(|e| matches!(e, BackendEvent::ContactUpdated(_))));
        assert!(has(|e| matches!(e, BackendEvent::DeliveryUpdate { .. })));
        assert!(has(|e| matches!(e, BackendEvent::Error(_))));
        assert!(has(
            |e| *e == BackendEvent::Connection(ConnectionState::Disconnected)
        ));
    }

    /// Nothing is dated after the moment it arrives, and the backlog is dated
    /// before the epoch.
    #[test]
    fn nothing_is_stamped_in_the_future() {
        for seed in 0..8 {
            for activity in [Duration::from_secs(4), Duration::from_millis(2)] {
                let config = MockConfig {
                    activity: Some(activity),
                    ..MockConfig::deterministic(seed)
                };
                let mut sim = Sim::new(&config, World::build());
                let epoch = sim.wall_clock_at(0).as_millis();
                let mut seen = 0;
                while seen < 3_000 {
                    let due = sim.next_due().expect("activity never runs out");
                    let now = epoch + due;
                    let backlog = due == CONNECT_DELAY;
                    for event in sim.pop_due(due) {
                        seen += 1;
                        let at = format!("seed {seed}, {activity:?}, {due} ms: {event:?}");
                        match &event {
                            BackendEvent::Message(m) => {
                                let sent = m.sent_at.as_millis();
                                assert!(sent <= now, "future message, {at}");
                                assert!(!backlog || sent < epoch, "backlog after epoch, {at}");
                            }
                            BackendEvent::Edited { edited_at, .. } => {
                                let edited = edited_at.as_millis();
                                assert!(edited <= now, "future edit, {at}");
                                assert!(
                                    !backlog || edited < epoch,
                                    "backlog edit after epoch, {at}"
                                );
                            }
                            BackendEvent::DeliveryUpdate { updated_at, .. } => {
                                let updated = updated_at.map_or(0, Timestamp::as_millis);
                                assert!(updated <= now, "future receipt, {at}");
                            }
                            BackendEvent::Connection(_)
                            | BackendEvent::Reaction { .. }
                            | BackendEvent::Deleted { .. }
                            | BackendEvent::Typing { .. }
                            | BackendEvent::ReadElsewhere(_)
                            | BackendEvent::ContactUpdated(_)
                            | BackendEvent::GroupUpdated(_)
                            | BackendEvent::Error(_) => {}
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn nothing_arrives_while_offline() {
        let mut connected = false;
        for event in run(&mut sim(3), 3_000) {
            if let BackendEvent::Connection(state) = event {
                connected = state == ConnectionState::Connected;
            } else {
                assert!(connected, "{event:?} while offline");
            }
        }
    }

    fn as_message(event: &BackendEvent) -> Option<&Message> {
        let BackendEvent::Message(m) = event else {
            return None;
        };
        Some(m)
    }

    #[test]
    fn message_refs_are_unique() {
        let events = run(&mut sim(5), 3_000);
        let refs: Vec<MessageRef> = events
            .iter()
            .filter_map(as_message)
            .map(Message::reference)
            .collect();
        let unique: std::collections::HashSet<_> = refs.iter().collect();
        assert_eq!(unique.len(), refs.len());
    }

    #[test]
    fn startup_connects_then_delivers_backlog_in_order() {
        let mut config = MockConfig::deterministic(9);
        config.activity = None;
        let mut sim = Sim::new(&config, World::build());
        let events = sim.pop_due(CONNECT_DELAY);
        let (connection, backlog) = events.split_at(2);
        assert_eq!(
            connection,
            [
                BackendEvent::Connection(ConnectionState::Connecting),
                BackendEvent::Connection(ConnectionState::Connected)
            ]
        );
        let times: Vec<Timestamp> = backlog
            .iter()
            .filter_map(as_message)
            .map(|m| m.sent_at)
            .collect();
        assert_eq!(times.len(), config.backlog);
        assert!(times.is_sorted());
        assert!(times.iter().all(|t| *t < sim.wall_clock_at(0)));
        assert_eq!(sim.next_due(), None, "no activity configured");
    }
}
