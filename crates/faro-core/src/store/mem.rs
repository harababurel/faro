//! The in-memory store engine (M13): the same operations as the SQLite engine
//! (`db.rs`), on plain Rust collections, for the web demo, where SQLite isn't
//! available. Nothing is persisted.
//!
//! It mirrors `db.rs` query by query, orderings included: senders compare as
//! their string form (SQL compares the TEXT column), searches lowercase only
//! ASCII (as SQLite's `lower()`), and a replaced reaction keeps its place (an
//! upsert keeps its row id). The store test suite runs against both engines.

use std::collections::{BTreeMap, HashMap};

use super::codec::{
    attachment_status_to_db, conversation_from_key, conversation_key, timestamp_to_db,
};
use super::{
    ConversationState, ConversationSummary, PROFILE_KEY, Profile, RecipientReceipt, Result,
    SearchHit, StoreError, StoreStats, StoredProfile, decide,
};
use crate::backend::BackendEvent;
use crate::domain::{
    AttachmentRef, Contact, ConversationId, DeliveryStatus, Direction, Group, Mention, Message,
    MessageBody, MessageRef, PhoneNumber, Quote, Reaction, ServiceId, Timestamp,
};

#[derive(Debug, Default)]
struct Recipient {
    contact_name: Option<String>,
    profile_name: Option<String>,
    phone: Option<PhoneNumber>,
    nickname: Option<String>,
}

#[derive(Debug, Default)]
struct Conversation {
    /// Groups only (a direct conversation's title is its contact's name).
    title: Option<String>,
    last_activity: Option<Timestamp>,
    unread: u32,
    /// `None` until set: reads back as the default state.
    state: Option<ConversationState>,
}

#[derive(Debug)]
struct Row {
    conversation: ConversationId,
    sender: ServiceId,
    /// `sender`'s string form, the sort key SQL uses.
    sender_key: String,
    sent_at: Timestamp,
    body: Option<String>,
    quote: Option<Quote>,
    edited_at: Option<Timestamp>,
    deleted: bool,
    direction: Direction,
    /// In row-id order: a replaced reaction keeps its place.
    reactions: Vec<Reaction>,
    attachments: Vec<AttachmentRef>,
    /// By start offset; the first mention at an offset wins.
    mentions: BTreeMap<usize, Mention>,
}

impl Row {
    /// `(sent_at, sender)`, SQL's row-value order for paging.
    fn point(&self) -> (Timestamp, &str) {
        (self.sent_at, &self.sender_key)
    }

    fn to_message(&self) -> Message {
        Message {
            conversation: self.conversation,
            sender: self.sender,
            sent_at: self.sent_at,
            direction: self.direction,
            body: self.body.clone().map(MessageBody::new),
            mentions: self.mentions.values().copied().collect(),
            quote: self.quote.clone(),
            attachments: self.attachments.clone(),
            reactions: self.reactions.clone(),
            edited_at: self.edited_at,
            deleted: self.deleted,
        }
    }
}

/// A per-recipient receipt: `(status, updated_at)`.
type Receipt = (DeliveryStatus, Timestamp);

#[derive(Debug, Default)]
pub(super) struct Mem {
    recipients: HashMap<ServiceId, Recipient>,
    /// By key (`ConversationId`'s string form), SQL's tie-break order.
    conversations: BTreeMap<String, Conversation>,
    /// By row id (insertion order).
    rows: BTreeMap<u64, Row>,
    by_ref: HashMap<MessageRef, u64>,
    next_id: u64,
    /// By message, then recipient's string form.
    receipts: HashMap<MessageRef, BTreeMap<String, (ServiceId, Receipt)>>,
    app_state: BTreeMap<String, String>,
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "the SQLite engine's signatures, so `call!` dispatches both engines"
)]
impl Mem {
    pub(super) fn apply(&mut self, event: BackendEvent) -> Result<()> {
        match event {
            BackendEvent::Message(message) => self.insert_message(&message),
            BackendEvent::DeliveryUpdate {
                message,
                status,
                by,
                updated_at,
            } => self.update_status(message, status, by, updated_at),
            BackendEvent::Reaction {
                target,
                sender,
                emoji,
                ..
            } => self.set_reaction(target, sender, emoji),
            BackendEvent::Edited {
                target,
                body,
                mentions,
                edited_at,
                ..
            } => self.edit(target, &body, &mentions, edited_at),
            BackendEvent::Deleted { target, .. } => self.delete(target),
            BackendEvent::ReadElsewhere(messages) => self.read_elsewhere(&messages).map(|_| ()),
            BackendEvent::ContactUpdated(contact) => self.upsert_contacts(&[contact]),
            BackendEvent::GroupUpdated(group) => self.upsert_groups(&[group]),
            BackendEvent::Connection(_) | BackendEvent::Typing { .. } | BackendEvent::Error(_) => {
                Ok(())
            }
        }
    }

    fn ensure_recipient(&mut self, id: ServiceId) {
        self.recipients.entry(id).or_default();
    }

    fn ensure_conversation(&mut self, id: ConversationId) -> String {
        if let ConversationId::Direct(peer) = id {
            self.ensure_recipient(peer);
        }
        let key = conversation_key(id);
        self.conversations.entry(key.clone()).or_default();
        key
    }

    fn row_mut(&mut self, target: MessageRef) -> Option<&mut Row> {
        let id = self.by_ref.get(&target)?;
        self.rows.get_mut(id)
    }

    pub(super) fn read_elsewhere(
        &mut self,
        messages: &[MessageRef],
    ) -> Result<Vec<(ConversationId, u32)>> {
        // The newest message read in each conversation, as `(sent_at, sender)`.
        let mut newest: HashMap<ConversationId, (Timestamp, String)> = HashMap::new();
        for message in messages {
            timestamp_to_db(message.sent_at)?;
            let Some(row) = self.by_ref.get(message).and_then(|id| self.rows.get(id)) else {
                tracing::debug!("read elsewhere: a message faro doesn't have");
                continue;
            };
            let point = (message.sent_at, message.sender.to_string());
            let entry = newest
                .entry(row.conversation)
                .or_insert_with(|| point.clone());
            if point > *entry {
                *entry = point;
            }
        }
        let mut cleared = Vec::new();
        for (conversation, (sent_at, sender)) in newest {
            let after = self
                .rows
                .values()
                .filter(|r| {
                    r.conversation == conversation
                        && r.direction == Direction::Incoming
                        && !r.deleted
                        && r.point() > (sent_at, sender.as_str())
                })
                .count();
            let after = u32::try_from(after).map_err(|_| StoreError::Corrupt("unread_count"))?;
            let Some(conv) = self.conversations.get_mut(&conversation_key(conversation)) else {
                continue;
            };
            if after >= conv.unread {
                continue;
            }
            cleared.push((conversation, conv.unread - after));
            conv.unread = after;
        }
        cleared.sort();
        Ok(cleared)
    }

    pub(super) fn upsert_contacts(&mut self, contacts: &[Contact]) -> Result<()> {
        for contact in contacts {
            let recipient = self.recipients.entry(contact.service_id).or_default();
            recipient.contact_name.clone_from(&contact.name);
            recipient.profile_name.clone_from(&contact.profile_name);
            recipient.phone.clone_from(&contact.phone);
            self.ensure_conversation(ConversationId::Direct(contact.service_id));
        }
        Ok(())
    }

    pub(super) fn upsert_groups(&mut self, groups: &[Group]) -> Result<()> {
        for group in groups {
            let id = ConversationId::Group(group.id);
            let key = self.ensure_conversation(id);
            if let Some(conv) = self.conversations.get_mut(&key) {
                conv.title = Some(group.title.clone());
            }
        }
        Ok(())
    }

    fn insert_message(&mut self, message: &Message) -> Result<()> {
        // The same checks as the SQL engine, in the same order: nothing changes
        // if one fails (the SQL transaction rolls back).
        timestamp_to_db(message.sent_at)?;
        if let Some(quote) = &message.quote {
            timestamp_to_db(quote.target.sent_at)?;
        }
        message.edited_at.map(timestamp_to_db).transpose()?;
        let reference = message.reference();
        if self.by_ref.contains_key(&reference) {
            // Already stored (a duplicate delivery): keep the first copy, but
            // refresh attachment pointers/dimensions if they were stored pre-M9.
            self.ensure_recipient(message.sender);
            self.ensure_conversation(message.conversation);
            if let Some(row) = self.row_mut(reference) {
                refresh_attachments(&mut row.attachments, &message.attachments);
            }
            return Ok(());
        }
        for attachment in &message.attachments {
            check_attachment(attachment)?;
        }
        for mention in &message.mentions {
            check_mention(mention)?;
        }
        self.ensure_recipient(message.sender);
        let key = self.ensure_conversation(message.conversation);
        let mut row = Row {
            conversation: message.conversation,
            sender: message.sender,
            sender_key: message.sender.to_string(),
            sent_at: message.sent_at,
            body: message.body.as_ref().map(|b| b.expose().to_owned()),
            quote: message.quote.clone(),
            edited_at: message.edited_at,
            deleted: message.deleted,
            direction: message.direction,
            reactions: Vec::new(),
            attachments: message.attachments.clone(),
            mentions: BTreeMap::new(),
        };
        for reaction in &message.reactions {
            set_reaction(&mut row.reactions, reaction.sender, &reaction.emoji);
        }
        insert_mentions(&mut row.mentions, &message.mentions);
        let id = self.next_id;
        self.next_id += 1;
        self.rows.insert(id, row);
        self.by_ref.insert(reference, id);
        if let Some(conv) = self.conversations.get_mut(&key) {
            conv.last_activity = Some(
                conv.last_activity
                    .map_or(message.sent_at, |last| last.max(message.sent_at)),
            );
            if message.direction == Direction::Incoming && !message.deleted {
                conv.unread = conv.unread.saturating_add(1);
            }
        }
        Ok(())
    }

    /// Receipts arrive late, twice or out of order: never move a status backwards.
    fn update_status(
        &mut self,
        message: MessageRef,
        status: DeliveryStatus,
        by: Option<ServiceId>,
        updated_at: Option<Timestamp>,
    ) -> Result<()> {
        timestamp_to_db(message.sent_at)?;
        let Some(row) = self
            .row_mut(message)
            .filter(|r| matches!(r.direction, Direction::Outgoing(_)))
        else {
            tracing::debug!("delivery update for an unknown message");
            return Ok(());
        };
        if let Direction::Outgoing(current) = row.direction {
            row.direction = Direction::Outgoing(current.update(status));
        }
        if let (Some(recipient), Some(at)) = (by, updated_at)
            && matches!(status, DeliveryStatus::Delivered | DeliveryStatus::Read)
        {
            timestamp_to_db(at)?;
            let receipts = self.receipts.entry(message).or_default();
            let entry = receipts
                .entry(recipient.to_string())
                .or_insert((recipient, (status, at)));
            let (old_status, old_at) = entry.1;
            let new_status = if old_status == DeliveryStatus::Read {
                DeliveryStatus::Read
            } else {
                status
            };
            let new_at =
                if old_status == DeliveryStatus::Read && status == DeliveryStatus::Delivered {
                    old_at
                } else {
                    at
                };
            entry.1 = (new_status, new_at);
        }
        Ok(())
    }

    fn set_reaction(
        &mut self,
        target: MessageRef,
        sender: ServiceId,
        emoji: Option<String>,
    ) -> Result<()> {
        timestamp_to_db(target.sent_at)?;
        let Some(row) = self.row_mut(target) else {
            tracing::debug!("reaction to an unknown message");
            return Ok(());
        };
        match emoji {
            Some(emoji) => set_reaction(&mut row.reactions, sender, &emoji),
            None => row.reactions.retain(|r| r.sender != sender),
        }
        Ok(())
    }

    /// Edits can arrive out of order; only a newer edit replaces the text
    /// (and its mentions).
    fn edit(
        &mut self,
        target: MessageRef,
        body: &MessageBody,
        mentions: &[Mention],
        edited_at: Timestamp,
    ) -> Result<()> {
        timestamp_to_db(edited_at)?;
        timestamp_to_db(target.sent_at)?;
        let Some(row) = self.row_mut(target) else {
            return Ok(());
        };
        if row.deleted || row.edited_at.is_some_and(|at| at >= edited_at) {
            return Ok(());
        }
        for mention in mentions {
            check_mention(mention)?;
        }
        row.body = Some(body.expose().to_owned());
        row.edited_at = Some(edited_at);
        row.mentions.clear();
        insert_mentions(&mut row.mentions, mentions);
        Ok(())
    }

    /// Deleted for everyone: keep a placeholder row, drop all content.
    fn delete(&mut self, target: MessageRef) -> Result<()> {
        timestamp_to_db(target.sent_at)?;
        let Some(row) = self.row_mut(target) else {
            return Ok(());
        };
        row.deleted = true;
        row.body = None;
        if let Some(quote) = &mut row.quote {
            quote.preview = None;
        }
        row.reactions.clear();
        row.attachments.clear();
        row.mentions.clear();
        self.receipts.remove(&target);
        Ok(())
    }

    /// Every conversation: pinned first (in pin order), then most recent activity,
    /// then empty ones.
    pub(super) fn conversations(&self) -> Result<Vec<ConversationSummary>> {
        let mut list: Vec<(&String, ConversationSummary)> = self
            .conversations
            .iter()
            .map(|(key, conv)| {
                let id = conversation_from_key(key)?;
                let (title, phone) = match id {
                    ConversationId::Group(_) => (conv.title.clone(), None),
                    ConversationId::Direct(service_id) => {
                        let recipient = self.recipients.get(&service_id);
                        let title = Contact {
                            service_id,
                            name: recipient.and_then(|r| r.contact_name.clone()),
                            profile_name: recipient.and_then(|r| r.profile_name.clone()),
                            phone: None,
                        }
                        .display_name()
                        .map(str::to_owned);
                        (title, recipient.and_then(|r| r.phone.clone()))
                    }
                };
                Ok((
                    key,
                    ConversationSummary {
                        id,
                        title,
                        phone,
                        last_activity: conv.last_activity,
                        unread: conv.unread,
                        state: conv.state.clone().unwrap_or_default(),
                    },
                ))
            })
            .collect::<Result<_>>()?;
        // ORDER BY pinned IS NULL, pinned, last_activity IS NULL, last_activity DESC, id
        list.sort_by(|(ka, a), (kb, b)| {
            (a.state.pinned.is_none(), a.state.pinned)
                .cmp(&(b.state.pinned.is_none(), b.state.pinned))
                .then_with(|| a.last_activity.is_none().cmp(&b.last_activity.is_none()))
                .then_with(|| b.last_activity.cmp(&a.last_activity))
                .then_with(|| ka.cmp(kb))
        });
        Ok(list.into_iter().map(|(_, summary)| summary).collect())
    }

    fn in_conversation(&self, conversation: ConversationId) -> impl Iterator<Item = &Row> {
        self.rows
            .values()
            .filter(move |r| r.conversation == conversation)
    }

    /// Up to `limit` messages older than `before` (newest page if `None`), oldest first.
    pub(super) fn messages(
        &self,
        conversation: ConversationId,
        before: Option<MessageRef>,
        limit: u32,
    ) -> Result<Vec<Message>> {
        let before = before
            .map(|b| timestamp_to_db(b.sent_at).map(|_| (b.sent_at, b.sender.to_string())))
            .transpose()?;
        let mut rows: Vec<&Row> = self
            .in_conversation(conversation)
            .filter(|r| {
                before
                    .as_ref()
                    .is_none_or(|(at, sender)| r.point() < (*at, sender.as_str()))
            })
            .collect();
        rows.sort_by(|a, b| b.point().cmp(&a.point()));
        rows.truncate(limit as usize);
        Ok(rows.into_iter().rev().map(Row::to_message).collect())
    }

    /// Up to `radius` messages at or before `target` plus up to `radius`
    /// messages after `target` in `conversation`, ordered oldest first (M12).
    pub(super) fn messages_around(
        &self,
        conversation: ConversationId,
        target: Timestamp,
        radius: u32,
    ) -> Result<Vec<Message>> {
        timestamp_to_db(target)?;
        let mut older: Vec<&Row> = self
            .in_conversation(conversation)
            .filter(|r| r.sent_at <= target)
            .collect();
        older.sort_by(|a, b| b.point().cmp(&a.point()));
        older.truncate(radius as usize);
        let mut newer: Vec<&Row> = self
            .in_conversation(conversation)
            .filter(|r| r.sent_at > target)
            .collect();
        newer.sort_by(|a, b| a.point().cmp(&b.point()));
        newer.truncate(radius as usize);
        Ok(older
            .into_iter()
            .rev()
            .chain(newer)
            .map(Row::to_message)
            .collect())
    }

    /// Search non-deleted messages matching all whitespace-separated tokens of
    /// `query` in `body` or attachment `transcript`, ordered newest first (M12).
    pub(super) fn search_messages(
        &self,
        conversation: Option<ConversationId>,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SearchHit>> {
        let tokens: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if tokens.is_empty() || limit == 0 {
            return Ok(vec![]);
        }
        let matches = |row: &Row| {
            let body = row.body.as_deref().unwrap_or("").to_ascii_lowercase();
            tokens.iter().all(|token| {
                body.contains(token.as_str())
                    || row.attachments.iter().any(|a| {
                        a.transcript
                            .as_deref()
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .contains(token.as_str())
                    })
            })
        };
        let mut rows: Vec<&Row> = self
            .rows
            .values()
            .filter(|r| !r.deleted && conversation.is_none_or(|c| r.conversation == c))
            .filter(|r| matches(r))
            .collect();
        // ORDER BY sent_at DESC, sender ASC
        rows.sort_by(|a, b| {
            b.sent_at
                .cmp(&a.sent_at)
                .then_with(|| a.sender_key.cmp(&b.sender_key))
        });
        rows.truncate(limit as usize);
        Ok(rows
            .into_iter()
            .map(|row| {
                let message = row.to_message();
                let matched_transcript = message.attachments.iter().find_map(|att| {
                    let text = att.transcript.as_ref()?;
                    let lower = text.to_lowercase();
                    tokens
                        .iter()
                        .any(|token| lower.contains(token))
                        .then(|| text.clone())
                });
                SearchHit {
                    conversation: message.conversation,
                    message,
                    matched_transcript,
                }
            })
            .collect())
    }

    /// Per-recipient delivery and read receipts for `message`, ordered by
    /// `updated_at` ascending (M12).
    pub(super) fn message_receipts(&self, message: MessageRef) -> Result<Vec<RecipientReceipt>> {
        timestamp_to_db(message.sent_at)?;
        let mut list: Vec<(&String, RecipientReceipt)> = self
            .receipts
            .get(&message)
            .into_iter()
            .flatten()
            .map(|(key, (recipient, (status, updated_at)))| {
                (
                    key,
                    RecipientReceipt {
                        recipient: *recipient,
                        status: *status,
                        updated_at: *updated_at,
                    },
                )
            })
            .collect();
        list.sort_by(|(ka, a), (kb, b)| a.updated_at.cmp(&b.updated_at).then_with(|| ka.cmp(kb)));
        Ok(list.into_iter().map(|(_, receipt)| receipt).collect())
    }

    pub(super) fn set_conversation_state(
        &mut self,
        conversation: ConversationId,
        state: &ConversationState,
    ) -> Result<()> {
        state.muted_until.map(timestamp_to_db).transpose()?;
        let key = self.ensure_conversation(conversation);
        if let Some(conv) = self.conversations.get_mut(&key) {
            conv.state = Some(state.clone());
        }
        Ok(())
    }

    /// Update the `index`-th attachment on `message` (download status, path,
    /// dimensions, audio waveform/duration, or transcript, D75, D80, D82).
    pub(super) fn set_attachment(
        &mut self,
        message: MessageRef,
        index: usize,
        attachment: &AttachmentRef,
    ) -> Result<()> {
        timestamp_to_db(message.sent_at)?;
        let Some(stored) = self
            .row_mut(message)
            .and_then(|row| row.attachments.get_mut(index))
        else {
            tracing::debug!("attachment update for an unknown message or index");
            return Ok(());
        };
        check_attachment(attachment)?;
        stored.size = attachment.size.or(stored.size);
        stored.width = attachment.width.or(stored.width);
        stored.height = attachment.height.or(stored.height);
        stored.status = attachment.status.clone();
        stored.voice_note |= attachment.voice_note;
        stored.duration_secs = attachment.duration_secs.or(stored.duration_secs);
        if attachment.waveform.is_some() {
            stored.waveform.clone_from(&attachment.waveform);
        }
        if attachment.transcript.is_some() {
            stored.transcript.clone_from(&attachment.transcript);
        }
        stored.transcript_source = attachment.transcript_source.or(stored.transcript_source);
        Ok(())
    }

    pub(super) fn mark_read(&mut self, conversation: ConversationId) -> Result<()> {
        if let Some(conv) = self.conversations.get_mut(&conversation_key(conversation)) {
            conv.unread = 0;
        }
        Ok(())
    }

    /// Mark outgoing messages still `sending` as `failed`; returns how many.
    pub(super) fn fail_unsent(&mut self) -> Result<u64> {
        let mut failed = 0;
        for row in self.rows.values_mut() {
            if row.direction == Direction::Outgoing(DeliveryStatus::Sending) {
                row.direction = Direction::Outgoing(DeliveryStatus::Failed);
                failed += 1;
            }
        }
        Ok(failed)
    }

    pub(super) fn claim_profile(&mut self, wanted: Profile) -> Result<()> {
        let stored = match self.app_state.get(PROFILE_KEY) {
            Some(text) => StoredProfile::Marked(text.clone()),
            None => StoredProfile::Unmarked {
                empty: self.app_state.is_empty()
                    && self.recipients.is_empty()
                    && self.conversations.is_empty()
                    && self.rows.is_empty(),
            },
        };
        if decide(wanted, stored)? {
            self.app_state
                .insert(PROFILE_KEY.to_owned(), wanted.as_str().to_owned());
        }
        Ok(())
    }

    pub(super) fn app_state(&self, key: &str) -> Result<Option<String>> {
        Ok(self.app_state.get(key).cloned())
    }

    pub(super) fn set_app_state(&mut self, key: &str, value: &str) -> Result<()> {
        self.app_state.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    pub(super) fn remove_app_state(&mut self, key: &str) -> Result<()> {
        self.app_state.remove(key);
        Ok(())
    }

    /// Everyone the user gave a nickname (D65).
    pub(super) fn nicknames(&self) -> Result<HashMap<ServiceId, String>> {
        Ok(self
            .recipients
            .iter()
            .filter_map(|(id, r)| r.nickname.clone().map(|n| (*id, n)))
            .collect())
    }

    /// Give someone a nickname, or take it away (`None`).
    pub(super) fn set_nickname(&mut self, id: ServiceId, nickname: Option<&str>) -> Result<()> {
        self.recipients.entry(id).or_default().nickname = nickname.map(str::to_owned);
        Ok(())
    }

    pub(super) fn stats(&self) -> Result<StoreStats> {
        Ok(StoreStats {
            conversations: self.conversations.len() as u64,
            messages: self.rows.len() as u64,
        })
    }
}

/// One reaction per sender: a new one replaces the old in place.
fn set_reaction(reactions: &mut Vec<Reaction>, sender: ServiceId, emoji: &str) {
    match reactions.iter_mut().find(|r| r.sender == sender) {
        Some(existing) => emoji.clone_into(&mut existing.emoji),
        None => reactions.push(Reaction {
            sender,
            emoji: emoji.to_owned(),
        }),
    }
}

fn insert_mentions(stored: &mut BTreeMap<usize, Mention>, mentions: &[Mention]) {
    for mention in mentions {
        stored.entry(mention.start).or_insert(*mention);
    }
}

/// A duplicate delivery may carry attachment pointers and dimensions that a
/// copy stored before M9 lacks: take them, keeping what's already known.
fn refresh_attachments(stored: &mut [AttachmentRef], fresh: &[AttachmentRef]) {
    for (old, new) in stored.iter_mut().zip(fresh) {
        if new.id.0.is_empty() {
            continue;
        }
        old.id.clone_from(&new.id);
        old.width = old.width.or(new.width);
        old.height = old.height.or(new.height);
        old.voice_note |= new.voice_note;
        old.duration_secs = old.duration_secs.or(new.duration_secs);
        if old.waveform.is_none() {
            old.waveform.clone_from(&new.waveform);
        }
    }
}

/// What the SQL engine refuses to store.
fn check_attachment(attachment: &AttachmentRef) -> Result<()> {
    attachment
        .size
        .map(|s| i64::try_from(s).map_err(|_| StoreError::OutOfRange("attachment size")))
        .transpose()?;
    attachment_status_to_db(&attachment.status)?;
    Ok(())
}

fn check_mention(mention: &Mention) -> Result<()> {
    for n in [mention.start, mention.len] {
        i64::try_from(n).map_err(|_| StoreError::OutOfRange("mention"))?;
    }
    Ok(())
}
