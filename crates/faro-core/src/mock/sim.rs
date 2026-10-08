//! The mock's brain: a seeded, synchronous event scheduler with no I/O.
//!
//! Time is "milliseconds since the mock started" ([`Millis`]). The async driver in
//! `mock.rs` sleeps until [`Sim::next_due`] and forwards what [`Sim::pop_due`]
//! returns. Follow-ups are scheduled relative to each entry's *due* time, never the
//! wall clock, so for a given seed (and the same commands at the same times) the
//! event sequence is identical even if the driver wakes up late.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, VecDeque};
use std::time::Duration;

use super::world::{ATTACHMENTS, REACTIONS, TEXTS, World, mock_attachment};
use super::{MockConfig, MockEpoch};
use crate::backend::{BackendError, BackendEvent, OutgoingMessage, Result};
use crate::domain::{
    ConnectionState, Contact, ConversationId, DeliveryStatus, Direction, Mention, Message,
    MessageBody, MessageRef, Quote, ServiceId, Timestamp, TypingState,
};

/// Milliseconds since the mock started.
pub(super) type Millis = u64;

/// Simulated time from "connecting" to "connected".
const CONNECT_DELAY: Millis = 400;
/// Backlog messages are spread over this window before the epoch.
const BACKLOG_SPAN: Millis = 3 * 24 * 60 * 60 * 1000;
/// Messages remembered as targets for reactions, replies, edits and deletes.
const RECENT_CAP: usize = 64;
/// Message refs remembered to keep `(sender, sent_at)` unique.
const ISSUED_CAP: usize = 256;
/// How often a group message mentions someone.
const MENTION_PERCENT: u32 = 15;
/// How often a message carries an attachment.
const ATTACHMENT_PERCENT: u32 = 12;

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
}

impl Sim {
    pub(super) fn new(config: &MockConfig, world: World) -> Self {
        let epoch_ms = match config.epoch {
            MockEpoch::Fixed(ts) => ts.as_millis(),
            MockEpoch::Now => wall_clock_ms(),
        };
        let mut sim = Self {
            rng: fastrand::Rng::with_seed(config.seed),
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

    /// The message reached the "server": report progress and maybe get a reply.
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
        self.remember(to, target, msg.body);
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

        let others = self.others_in(to);
        if self.chance(60)
            && let Some(sender) = self.rng.choice(others)
        {
            let quote = self.chance(30).then(|| self.quote_of(target)).flatten();
            let start = read_at + self.rng.u64(1_000..=3_000);
            self.schedule_incoming(start, to, sender, quote);
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

    /// Someone starts typing at `start` and sends a message a few seconds later.
    fn schedule_incoming(
        &mut self,
        start: Millis,
        conversation: ConversationId,
        sender: ServiceId,
        quote: Option<Quote>,
    ) {
        let typing = |state| BackendEvent::Typing {
            conversation,
            sender,
            state,
        };
        let sent = start + self.rng.u64(1_000..=5_000);
        let sent_at = self.stamp(sender, sent);
        let message = self.message(conversation, sender, sent_at, Direction::Incoming, quote);
        self.emit(start, typing(TypingState::Started));
        self.emit(sent, BackendEvent::Message(message));
        self.emit(sent, typing(TypingState::Stopped));
    }

    /// Messages "queued on the server" while faro was closed: delivered right after
    /// connecting, with timestamps from the past few days.
    fn schedule_backlog(&mut self, count: usize) {
        let mut offsets: Vec<Millis> = (0..count)
            .map(|_| self.rng.u64(60_000..=BACKLOG_SPAN))
            .collect();
        offsets.sort_unstable_by(|a, b| b.cmp(a));
        let me = self.world.me();
        for offset in offsets {
            let Some(conversation) = self.rng.choice(self.world.active.clone()) else {
                return;
            };
            let (sender, direction) = if self.chance(25) {
                (me, Direction::Outgoing(DeliveryStatus::Read))
            } else {
                let others = self.others_in(conversation);
                match self.rng.choice(others) {
                    Some(sender) => (sender, Direction::Incoming),
                    None => continue,
                }
            };
            let sent_at = self.stamp_wall(sender, self.epoch_ms.saturating_sub(offset));
            let message = self.message(conversation, sender, sent_at, direction, None);
            self.emit(CONNECT_DELAY, BackendEvent::Message(message));
        }
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

    fn chat(&mut self, now: Millis) -> Option<()> {
        let conversation = self.rng.choice(self.world.active.clone())?;
        let sender = self.rng.choice(self.others_in(conversation))?;
        self.schedule_incoming(now, conversation, sender, None);
        Some(())
    }

    fn reply(&mut self, now: Millis) -> Option<()> {
        let (conversation, target) = self.pick_recent(|_| true)?;
        let sender = self.rng.choice(self.others_in(conversation))?;
        let quote = self.quote_of(target);
        self.schedule_incoming(now, conversation, sender, quote);
        Some(())
    }

    fn react(&mut self, now: Millis) -> Option<()> {
        let (conversation, target) = self.pick_recent(|_| true)?;
        let mut reactors = self.others_in(conversation);
        reactors.retain(|s| *s != target.sender);
        let sender = self.rng.choice(reactors)?;
        let emoji = self.rng.choice(REACTIONS).map(|e| (*e).to_owned());
        let reaction = BackendEvent::Reaction {
            conversation,
            target,
            sender,
            emoji,
        };
        self.emit(now, reaction);
        Some(())
    }

    /// Sent by the owner from their phone: arrives as an outgoing message.
    fn sync_message(&mut self, now: Millis) -> Option<()> {
        let conversation = self.rng.choice(self.world.active.clone())?;
        let me = self.world.me();
        let sent_at = self.stamp(me, now);
        let direction = Direction::Outgoing(DeliveryStatus::Sent);
        let message = self.message(conversation, me, sent_at, direction, None);
        let target = message.reference();
        self.emit(now, BackendEvent::Message(message));
        self.schedule_receipts(now, conversation, target);
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

    fn edit(&mut self, now: Millis) -> Option<()> {
        let me = self.world.me();
        let (conversation, target) = self.pick_recent(|r| r.target.sender != me)?;
        let edited = BackendEvent::Edited {
            conversation,
            target,
            body: self.text(),
            mentions: vec![],
            edited_at: self.wall_clock_at(now),
        };
        self.emit(now, edited);
        Some(())
    }

    fn delete(&mut self, now: Millis) -> Option<()> {
        let me = self.world.me();
        let (conversation, target) = self.pick_recent(|r| r.target.sender != me)?;
        let deleted = BackendEvent::Deleted {
            conversation,
            target,
        };
        self.emit(now, deleted);
        Some(())
    }

    fn profile_change(&mut self, now: Millis) -> Option<()> {
        let direct: Vec<ServiceId> = self
            .world
            .active
            .iter()
            .filter_map(|c| match c {
                ConversationId::Direct(id) => Some(*id),
                ConversationId::Group(_) => None,
            })
            .collect();
        let id = self.rng.choice(direct)?;
        let contact = self
            .world
            .contacts
            .iter_mut()
            .find(|c| c.service_id == id)?;
        toggle_sparkle(contact);
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
                    self.remember(m.conversation, m.reference(), body.clone());
                }
            }
            BackendEvent::Edited { target, body, .. } => {
                if let Some(r) = self.recent.iter_mut().find(|r| r.target == *target) {
                    r.body = body.clone();
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

    fn remember(&mut self, conversation: ConversationId, target: MessageRef, body: MessageBody) {
        if self.recent.len() >= RECENT_CAP {
            self.recent.pop_front();
        }
        self.recent.push_back(Recent {
            conversation,
            target,
            body,
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

    /// A unique `sent_at` for `sender` at simulated time `at`.
    fn stamp(&mut self, sender: ServiceId, at: Millis) -> Timestamp {
        self.stamp_wall(sender, self.epoch_ms.saturating_add(at))
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

    fn message(
        &mut self,
        conversation: ConversationId,
        sender: ServiceId,
        sent_at: Timestamp,
        direction: Direction,
        quote: Option<Quote>,
    ) -> Message {
        let text = self.text();
        let (body, mentions) = match self.mentioned(conversation, sender) {
            Some(who) => (
                MessageBody::new(format!("\u{FFFC} {}", text.expose())),
                vec![Mention {
                    who,
                    start: 0,
                    len: '\u{FFFC}'.len_utf8(),
                }],
            ),
            None => (text, vec![]),
        };
        let attachments = if self.chance(ATTACHMENT_PERCENT) {
            mock_attachment(self.rng.usize(0..ATTACHMENTS.len()))
                .into_iter()
                .collect()
        } else {
            vec![]
        };
        Message {
            conversation,
            sender,
            sent_at,
            direction,
            body: Some(body),
            mentions,
            quote,
            attachments,
            reactions: vec![],
            edited_at: None,
            deleted: false,
        }
    }

    /// Now and then, someone in a group to mention (D72); this account half
    /// the time, so mentions-only notifications can be tried.
    fn mentioned(&mut self, conversation: ConversationId, sender: ServiceId) -> Option<ServiceId> {
        if !matches!(conversation, ConversationId::Group(_)) || !self.chance(MENTION_PERCENT) {
            return None;
        }
        let me = self.world.me();
        if sender != me && self.chance(50) {
            return Some(me);
        }
        let others: Vec<ServiceId> = self
            .others_in(conversation)
            .into_iter()
            .filter(|s| *s != sender)
            .collect();
        self.rng.choice(others)
    }

    fn text(&mut self) -> MessageBody {
        MessageBody::new(self.rng.choice(TEXTS).copied().unwrap_or("…"))
    }

    fn chance(&mut self, percent: u32) -> bool {
        self.rng.u32(0..100) < percent
    }

    /// Uniform in `[mean / 2, mean * 3 / 2]`.
    fn jitter(&mut self, mean: Millis) -> Millis {
        let half = mean / 2;
        self.rng.u64(half..=mean + half).max(1)
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

/// A visible, reversible profile change.
fn toggle_sparkle(contact: &mut Contact) {
    const SPARKLE: &str = " ✨";
    let name = contact.profile_name.take().unwrap_or_default();
    contact.profile_name = Some(match name.strip_suffix(SPARKLE) {
        Some(plain) => plain.to_owned(),
        None => format!("{name}{SPARKLE}"),
    });
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
