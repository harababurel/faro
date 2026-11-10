//! SQL for the store. Owned by the store task (`store.rs`); nothing else touches
//! the connection (D22).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{ConnectOptions as _, Connection as _};

use super::codec::{
    conversation_from_key, conversation_key, count_from_db, direction_from_db, direction_to_db,
    optional_timestamp_from_db, service_id_from_db, status_from_db, status_to_db,
    system_event_from_db, system_event_to_db, timestamp_from_db, timestamp_to_db,
    verification_from_db,
};
use super::{
    ConversationState, ConversationSummary, ExpiredSweep, NotifyLevel, PROFILE_KEY, Profile,
    RecipientReceipt, Result, SearchHit, StoreError, StoreStats, StoredProfile, decide,
};
use crate::backend::{BackendEvent, IdentityUpdate, MessageRequestAction};
use crate::domain::{
    CallState, Contact, ConversationId, DeliveryStatus, Direction, Group, IdentityRecord,
    LinkPreview, Mention, Message, MessageBody, MessageRef, PhoneNumber, Quote, Reaction,
    ServiceId, SystemEvent, Timestamp, VerificationState,
};

mod attachments;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

type MessageRow = (
    i64,
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    bool,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

pub(super) struct Db {
    conn: SqliteConnection,
}

impl Db {
    /// Connect and bring the schema up to date.
    pub(super) async fn open(options: &SqliteConnectOptions) -> Result<Self> {
        let mut conn = options.connect().await?;
        MIGRATOR.run(&mut conn).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS link_previews (
                 message_id  INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
                 idx         INTEGER NOT NULL,
                 url         TEXT NOT NULL,
                 title       TEXT,
                 description TEXT,
                 PRIMARY KEY (message_id, idx)
             ) STRICT",
        )
        .execute(&mut conn)
        .await?;
        Ok(Self { conn })
    }

    /// Refuse the existing database at `path` if it belongs to the other
    /// profile, without changing it: read-only, no journal mode switch, no
    /// migrations (D87). [`Db::claim_profile`] marks it after migrating.
    pub(super) async fn check_profile(path: &Path, wanted: Profile) -> Result<()> {
        decide(wanted, peek_profile(path).await?)?;
        Ok(())
    }

    pub(super) async fn close(self) -> Result<()> {
        self.conn.close().await.map_err(StoreError::from)
    }

    /// Persist what a backend reported. Transient events (connection, typing,
    /// errors) aren't stored.
    pub(super) async fn apply(&mut self, event: BackendEvent) -> Result<()> {
        match event {
            BackendEvent::Message(message) => self.insert_message(&message).await,
            BackendEvent::DeliveryUpdate {
                message,
                status,
                by,
                updated_at,
            } => self.update_status(message, status, by, updated_at).await,
            BackendEvent::Reaction {
                target,
                sender,
                emoji,
                ..
            } => self.set_reaction(target, sender, emoji.as_deref()).await,
            BackendEvent::Edited {
                target,
                body,
                mentions,
                edited_at,
                ..
            } => self.edit(target, &body, &mentions, edited_at).await,
            BackendEvent::Deleted { target, .. } => self.delete(target).await,
            BackendEvent::ReadElsewhere(messages) => {
                self.read_elsewhere(&messages).await.map(|_| ())
            }
            BackendEvent::ContactUpdated(contact) => self.upsert_contacts(&[contact]).await,
            BackendEvent::IdentityUpdated(update) => self.upsert_identities(&[update]).await,
            BackendEvent::GroupUpdated(group) => self.upsert_groups(&[group]).await,
            BackendEvent::DisappearingTimer {
                conversation,
                seconds,
            } => self.set_disappearing_timer(conversation, seconds).await,
            BackendEvent::BlockedSynced { conversations } => {
                self.sync_blocked(&conversations).await
            }
            BackendEvent::MessageRequestResponded {
                conversation,
                action,
            } => {
                self.apply_message_request_response(conversation, action)
                    .await
            }
            BackendEvent::Connection(_) | BackendEvent::Typing { .. } | BackendEvent::Error(_) => {
                Ok(())
            }
        }
    }

    /// Messages read on another device (D70): in each conversation, only
    /// incoming messages after the newest one read can still be unread, so
    /// the count drops to at most that many. Returns how much each
    /// conversation's count dropped (only those that did).
    pub(super) async fn read_elsewhere(
        &mut self,
        messages: &[MessageRef],
    ) -> Result<Vec<(ConversationId, u32)>> {
        let mut tx = self.conn.begin().await?;
        // The newest message read in each conversation, as `(sent_at, sender)`.
        let mut newest: HashMap<String, (i64, String)> = HashMap::new();
        for message in messages {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT conversation_id FROM messages WHERE sender = ?1 AND sent_at = ?2",
            )
            .bind(message.sender.to_string())
            .bind(timestamp_to_db(message.sent_at)?)
            .fetch_optional(&mut *tx)
            .await?;
            let Some((conversation,)) = row else {
                tracing::debug!("read elsewhere: a message faro doesn't have");
                continue;
            };
            let point = (
                timestamp_to_db(message.sent_at)?,
                message.sender.to_string(),
            );
            let entry = newest.entry(conversation).or_insert_with(|| point.clone());
            if point > *entry {
                *entry = point;
            }
        }
        let now_db = timestamp_to_db(now())?;
        let mut cleared = Vec::new();
        for (key, (sent_at, sender)) in newest {
            sqlx::query(
                "UPDATE messages SET expire_started_at = ?1
                 WHERE conversation_id = ?2
                   AND (sent_at, sender) <= (?3, ?4)
                   AND expires_in_secs IS NOT NULL
                   AND expire_started_at IS NULL",
            )
            .bind(now_db)
            .bind(&key)
            .bind(sent_at)
            .bind(&sender)
            .execute(&mut *tx)
            .await?;
            let (unread, after): (i64, i64) = sqlx::query_as(
                "SELECT c.unread_count,
                        (SELECT count(*) FROM messages m
                         WHERE m.conversation_id = c.id AND m.direction = 'incoming'
                           AND m.deleted = 0 AND m.system_event IS NULL
                           AND (m.sent_at, m.sender) > (?2, ?3))
                 FROM conversations c WHERE c.id = ?1",
            )
            .bind(&key)
            .bind(sent_at)
            .bind(&sender)
            .fetch_one(&mut *tx)
            .await?;
            if after >= unread {
                continue;
            }
            sqlx::query("UPDATE conversations SET unread_count = ?1 WHERE id = ?2")
                .bind(after)
                .bind(&key)
                .execute(&mut *tx)
                .await?;
            let dropped = count_from_db(unread - after, "unread_count")?;
            cleared.push((conversation_from_key(&key)?, dropped));
        }
        tx.commit().await?;
        cleared.sort();
        Ok(cleared)
    }

    /// Insert or replace contacts; each gets a direct conversation (possibly empty).
    pub(super) async fn upsert_contacts(&mut self, contacts: &[Contact]) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        for contact in contacts {
            sqlx::query(
                "INSERT INTO recipients (service_id, contact_name, profile_name, phone)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (service_id) DO UPDATE SET
                     contact_name = excluded.contact_name,
                     profile_name = coalesce(excluded.profile_name, recipients.profile_name),
                     phone = excluded.phone",
            )
            .bind(contact.service_id.to_string())
            .bind(contact.name.as_deref())
            .bind(contact.profile_name.as_deref())
            .bind(contact.phone.as_ref().map(PhoneNumber::expose))
            .execute(&mut *tx)
            .await?;
            ensure_conversation(&mut tx, ConversationId::Direct(contact.service_id)).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Insert or replace groups (title and members).
    pub(super) async fn upsert_groups(&mut self, groups: &[Group]) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        for group in groups {
            let key = conversation_key(ConversationId::Group(group.id));
            sqlx::query(
                "INSERT INTO conversations (id, kind, title) VALUES (?1, 'group', ?2)
                 ON CONFLICT (id) DO UPDATE SET title = excluded.title",
            )
            .bind(&key)
            .bind(&group.title)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM group_members WHERE group_id = ?1")
                .bind(&key)
                .execute(&mut *tx)
                .await?;
            for member in &group.members {
                sqlx::query(
                    "INSERT OR IGNORE INTO group_members (group_id, service_id) VALUES (?1, ?2)",
                )
                .bind(&key)
                .bind(member.to_string())
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    async fn insert_message(&mut self, message: &Message) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        ensure_recipient(&mut tx, message.sender).await?;
        let conversation = ensure_conversation(&mut tx, message.conversation).await?;
        let (direction, status) = direction_to_db(message.direction);
        let sent_at = timestamp_to_db(message.sent_at)?;
        let quote_sent_at = message
            .quote
            .as_ref()
            .map(|q| timestamp_to_db(q.target.sent_at))
            .transpose()?;
        let edited_at = message.edited_at.map(timestamp_to_db).transpose()?;
        let expires_in_secs = message.expires_in_secs.filter(|&s| s > 0);
        let expire_started_at = expires_in_secs.and_then(|_| {
            message.expire_started_at.or_else(|| {
                matches!(message.direction, Direction::Outgoing(_)).then_some(message.sent_at)
            })
        });
        let expire_started_at_db = expire_started_at.map(timestamp_to_db).transpose()?;
        if coalesce_system_event(&mut tx, &conversation, message, sent_at).await? {
            tx.commit().await?;
            return Ok(());
        }
        let system_event_db = message.system_event.as_ref().map(system_event_to_db);
        let inserted = sqlx::query(
            "INSERT INTO messages (conversation_id, sender, sent_at, received_at, body,
                 quote_sender, quote_sent_at, quote_preview, edited_at, deleted, direction, status,
                 expires_in_secs, expire_started_at, system_event)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT (sender, sent_at) DO NOTHING",
        )
        .bind(&conversation)
        .bind(message.sender.to_string())
        .bind(sent_at)
        .bind(timestamp_to_db(now())?)
        .bind(message.body.as_ref().map(MessageBody::expose))
        .bind(message.quote.as_ref().map(|q| q.target.sender.to_string()))
        .bind(quote_sent_at)
        .bind(
            message
                .quote
                .as_ref()
                .and_then(|q| q.preview.as_ref())
                .map(MessageBody::expose),
        )
        .bind(edited_at)
        .bind(message.deleted)
        .bind(direction)
        .bind(status)
        .bind(expires_in_secs.map(i64::from))
        .bind(expire_started_at_db)
        .bind(system_event_db)
        .execute(&mut *tx)
        .await?;

        if inserted.rows_affected() == 0 {
            refresh_duplicate_message(
                &mut tx,
                message,
                sent_at,
                expires_in_secs,
                expire_started_at_db,
            )
            .await?;
            tx.commit().await?;
            return Ok(());
        }
        let id = inserted.last_insert_rowid();
        for reaction in &message.reactions {
            insert_reaction(&mut tx, id, reaction.sender, &reaction.emoji).await?;
        }
        for attachment in &message.attachments {
            attachments::insert(&mut tx, id, attachment).await?;
        }
        insert_mentions(&mut tx, id, &message.mentions).await?;
        insert_link_previews(&mut tx, id, &message.previews).await?;
        let unread = i64::from(
            message.direction == Direction::Incoming
                && !message.deleted
                && message.system_event.is_none(),
        );
        sqlx::query(
            "UPDATE conversations
             SET last_activity = max(coalesce(last_activity, ?1), ?1),
                 unread_count = unread_count + ?2,
                 disappearing_timer = coalesce(?3, disappearing_timer)
             WHERE id = ?4",
        )
        .bind(sent_at)
        .bind(unread)
        .bind(expires_in_secs.map(i64::from))
        .bind(&conversation)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Receipts arrive late, twice or out of order: never move a status backwards.
    async fn update_status(
        &mut self,
        message: MessageRef,
        status: DeliveryStatus,
        by: Option<ServiceId>,
        updated_at: Option<Timestamp>,
    ) -> Result<()> {
        let sender = message.sender.to_string();
        let sent_at = timestamp_to_db(message.sent_at)?;
        let row: Option<(i64, Option<String>)> = sqlx::query_as(
            "SELECT id, status FROM messages
             WHERE sender = ?1 AND sent_at = ?2 AND direction = 'outgoing'",
        )
        .bind(&sender)
        .bind(sent_at)
        .fetch_optional(&mut self.conn)
        .await?;
        let Some((id, current)) = row else {
            tracing::debug!("delivery update for an unknown message");
            return Ok(());
        };
        let current = current
            .as_deref()
            .ok_or(StoreError::Corrupt("status"))
            .and_then(status_from_db)?;
        let updated = current.update(status);
        if updated != current {
            sqlx::query("UPDATE messages SET status = ?1 WHERE id = ?2")
                .bind(status_to_db(updated))
                .bind(id)
                .execute(&mut self.conn)
                .await?;
        }
        if let (Some(recipient), Some(at)) = (by, updated_at)
            && matches!(status, DeliveryStatus::Delivered | DeliveryStatus::Read)
        {
            sqlx::query(
                "INSERT INTO message_receipts (sender, sent_at, recipient, status, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(sender, sent_at, recipient) DO UPDATE SET
                     status = CASE
                         WHEN message_receipts.status = 'read' THEN 'read'
                         ELSE excluded.status
                     END,
                     updated_at = CASE
                         WHEN message_receipts.status = 'read' AND excluded.status = 'delivered'
                             THEN message_receipts.updated_at
                         ELSE excluded.updated_at
                     END",
            )
            .bind(&sender)
            .bind(sent_at)
            .bind(recipient.to_string())
            .bind(status_to_db(status))
            .bind(timestamp_to_db(at)?)
            .execute(&mut self.conn)
            .await?;
        }
        Ok(())
    }

    async fn set_reaction(
        &mut self,
        target: MessageRef,
        sender: ServiceId,
        emoji: Option<&str>,
    ) -> Result<()> {
        let Some(id) = message_id(&mut self.conn, target).await? else {
            tracing::debug!("reaction to an unknown message");
            return Ok(());
        };
        if let Some(emoji) = emoji {
            return insert_reaction(&mut self.conn, id, sender, emoji).await;
        }
        sqlx::query("DELETE FROM reactions WHERE message_id = ?1 AND sender = ?2")
            .bind(id)
            .bind(sender.to_string())
            .execute(&mut self.conn)
            .await?;
        Ok(())
    }

    /// Edits can arrive out of order; only a newer edit replaces the text
    /// (and its mentions).
    async fn edit(
        &mut self,
        target: MessageRef,
        body: &MessageBody,
        mentions: &[Mention],
        edited_at: Timestamp,
    ) -> Result<()> {
        let edited_at = timestamp_to_db(edited_at)?;
        let mut tx = self.conn.begin().await?;
        let updated: Option<(i64,)> = sqlx::query_as(
            "UPDATE messages SET body = ?1, edited_at = ?2
             WHERE sender = ?3 AND sent_at = ?4 AND deleted = 0
               AND (edited_at IS NULL OR edited_at < ?2)
             RETURNING id",
        )
        .bind(body.expose())
        .bind(edited_at)
        .bind(target.sender.to_string())
        .bind(timestamp_to_db(target.sent_at)?)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((id,)) = updated {
            sqlx::query("DELETE FROM mentions WHERE message_id = ?1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            insert_mentions(&mut tx, id, mentions).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Deleted for everyone: keep a placeholder row, drop all content.
    async fn delete(&mut self, target: MessageRef) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        let Some(id) = message_id(&mut tx, target).await? else {
            return Ok(());
        };
        sqlx::query(
            "UPDATE messages SET deleted = 1, body = NULL, quote_preview = NULL WHERE id = ?1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        for table in [
            "DELETE FROM reactions WHERE message_id = ?1",
            "DELETE FROM attachments WHERE message_id = ?1",
            "DELETE FROM mentions WHERE message_id = ?1",
            "DELETE FROM link_previews WHERE message_id = ?1",
        ] {
            sqlx::query(table).bind(id).execute(&mut *tx).await?;
        }
        sqlx::query("DELETE FROM message_receipts WHERE sender = ?1 AND sent_at = ?2")
            .bind(target.sender.to_string())
            .bind(timestamp_to_db(target.sent_at)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every conversation: pinned first (in pin order), then most recent activity,
    /// then empty ones. Filtering (hidden, archived, empty) is the UI's job.
    pub(super) async fn conversations(&mut self) -> Result<Vec<ConversationSummary>> {
        #[derive(sqlx::FromRow)]
        struct Row {
            id: String,
            title: Option<String>,
            last_activity: Option<i64>,
            unread_count: i64,
            contact_name: Option<String>,
            profile_name: Option<String>,
            phone: Option<String>,
            hidden: Option<bool>,
            archived: Option<bool>,
            pinned: Option<i64>,
            muted_until: Option<i64>,
            draft: Option<String>,
            allowed: Option<bool>,
            notify: Option<String>,
            remote_transcription: Option<bool>,
            message_request: Option<bool>,
            blocked: Option<bool>,
            disappearing_timer: Option<i64>,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT c.id, c.title, c.last_activity, c.unread_count,
                    r.contact_name, r.profile_name, r.phone,
                    s.hidden, s.archived, s.pinned, s.muted_until, s.draft, s.allowed,
                    s.notify, s.remote_transcription, s.message_request, s.blocked,
                    c.disappearing_timer
             FROM conversations c
             LEFT JOIN recipients r ON r.service_id = c.peer
             LEFT JOIN conversation_state s ON s.conversation_id = c.id
             ORDER BY s.pinned IS NULL, s.pinned,
                      c.last_activity IS NULL, c.last_activity DESC, c.id",
        )
        .fetch_all(&mut self.conn)
        .await?;

        let mut summaries = rows
            .into_iter()
            .map(|row| {
                let id = conversation_from_key(&row.id)?;
                let title = match id {
                    ConversationId::Group(_) => row.title,
                    ConversationId::Direct(service_id) => Contact {
                        service_id,
                        name: row.contact_name,
                        profile_name: row.profile_name,
                        phone: None,
                    }
                    .display_name()
                    .map(str::to_owned),
                };
                Ok(ConversationSummary {
                    id,
                    title,
                    phone: row.phone.map(PhoneNumber::new),
                    last_activity: optional_timestamp_from_db(row.last_activity, "last_activity")?,
                    unread: count_from_db(row.unread_count, "unread_count")?,
                    disappearing_timer: row
                        .disappearing_timer
                        .map(|t| count_from_db(t, "disappearing_timer"))
                        .transpose()?,
                    state: ConversationState {
                        hidden: row.hidden.unwrap_or(false),
                        archived: row.archived.unwrap_or(false),
                        pinned: row.pinned.map(|p| count_from_db(p, "pinned")).transpose()?,
                        muted_until: optional_timestamp_from_db(row.muted_until, "muted_until")?,
                        draft: row.draft.map(MessageBody::new),
                        allowed: row.allowed.unwrap_or(false),
                        notify: row
                            .notify
                            .map(|n| NotifyLevel::parse(&n).ok_or(StoreError::Corrupt("notify")))
                            .transpose()?
                            .unwrap_or_default(),
                        remote_transcription: super::RemoteTranscription::from_bool(
                            row.remote_transcription.unwrap_or(false),
                        ),
                        request: row.message_request.unwrap_or(false),
                        blocked: row.blocked.unwrap_or(false),
                    },
                    last_message: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut latest = self.last_messages().await?;
        for summary in &mut summaries {
            summary.last_message = latest.remove(&summary.id);
        }
        Ok(summaries)
    }

    /// Each conversation's newest message (D154): one indexed lookup per
    /// conversation, then their reactions, attachments and mentions.
    async fn last_messages(&mut self) -> Result<HashMap<ConversationId, Message>> {
        // The conversation's key, then a `MessageRow`.
        type Row = (
            String,
            i64,
            String,
            i64,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<i64>,
            bool,
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<String>,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT c.id, m.id, m.sender, m.sent_at, m.body,
                        m.quote_sender, m.quote_sent_at, m.quote_preview,
                        m.edited_at, m.deleted, m.direction, m.status,
                        m.expires_in_secs, m.expire_started_at, m.system_event
                 FROM conversations c
                 JOIN messages m ON m.id = (
                     SELECT id FROM messages
                     WHERE conversation_id = c.id
                     ORDER BY sent_at DESC, sender DESC
                     LIMIT 1)",
        )
        .fetch_all(&mut self.conn)
        .await?;
        let mut latest = HashMap::with_capacity(rows.len());
        for row in rows {
            let conversation = conversation_from_key(&row.0)?;
            let message_row = (
                row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, row.10, row.11,
                row.12, row.13, row.14,
            );
            let (id, message) = decode_message_row(conversation, message_row)?;
            // One message at a time: the row IDs of different conversations'
            // newest messages are far apart, and `attach_extras` scans the
            // range between the lowest and the highest.
            let mut one = [message];
            self.attach_extras(&[id], &mut one).await?;
            let [message] = one;
            latest.insert(conversation, message);
        }
        Ok(latest)
    }

    /// Up to `limit` messages older than `before` (newest page if `None`), oldest first.
    pub(super) async fn messages(
        &mut self,
        conversation: ConversationId,
        before: Option<MessageRef>,
        limit: u32,
    ) -> Result<Vec<Message>> {
        let key = conversation_key(conversation);
        let rows: Vec<MessageRow> = sqlx::query_as(
            "SELECT id, sender, sent_at, body, quote_sender, quote_sent_at, quote_preview,
                    edited_at, deleted, direction, status, expires_in_secs, expire_started_at,
                    system_event
             FROM messages
             WHERE conversation_id = ?1
               AND (?2 IS NULL OR (sent_at, sender) < (?2, ?3))
             ORDER BY sent_at DESC, sender DESC
             LIMIT ?4",
        )
        .bind(&key)
        .bind(before.map(|b| timestamp_to_db(b.sent_at)).transpose()?)
        .bind(before.map(|b| b.sender.to_string()))
        .bind(i64::from(limit))
        .fetch_all(&mut self.conn)
        .await?;

        let mut ids = Vec::with_capacity(rows.len());
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows.into_iter().rev() {
            let (id, message) = decode_message_row(conversation, row)?;
            ids.push(id);
            messages.push(message);
        }
        self.attach_extras(&ids, &mut messages).await?;
        Ok(messages)
    }

    /// Up to `radius` messages at or before `target` plus up to `radius`
    /// messages after `target` in `conversation`, ordered oldest first (M12).
    pub(super) async fn messages_around(
        &mut self,
        conversation: ConversationId,
        target: Timestamp,
        radius: u32,
    ) -> Result<Vec<Message>> {
        let key = conversation_key(conversation);
        let target_db = timestamp_to_db(target)?;
        let limit = i64::from(radius);
        let older: Vec<MessageRow> = sqlx::query_as(
            "SELECT id, sender, sent_at, body, quote_sender, quote_sent_at, quote_preview,
                    edited_at, deleted, direction, status, expires_in_secs, expire_started_at,
                    system_event
             FROM messages
             WHERE conversation_id = ?1 AND sent_at <= ?2
             ORDER BY sent_at DESC, sender DESC
             LIMIT ?3",
        )
        .bind(&key)
        .bind(target_db)
        .bind(limit)
        .fetch_all(&mut self.conn)
        .await?;
        let newer: Vec<MessageRow> = sqlx::query_as(
            "SELECT id, sender, sent_at, body, quote_sender, quote_sent_at, quote_preview,
                    edited_at, deleted, direction, status, expires_in_secs, expire_started_at,
                    system_event
             FROM messages
             WHERE conversation_id = ?1 AND sent_at > ?2
             ORDER BY sent_at ASC, sender ASC
             LIMIT ?3",
        )
        .bind(&key)
        .bind(target_db)
        .bind(limit)
        .fetch_all(&mut self.conn)
        .await?;

        let total = older.len().saturating_add(newer.len());
        let mut ids = Vec::with_capacity(total);
        let mut messages = Vec::with_capacity(total);
        for row in older.into_iter().rev().chain(newer) {
            let (id, message) = decode_message_row(conversation, row)?;
            ids.push(id);
            messages.push(message);
        }
        self.attach_extras(&ids, &mut messages).await?;
        Ok(messages)
    }

    /// Walk up the quote chain from `target` in `conversation` to the root
    /// message (or missing root reference) and return the root plus every
    /// direct and transitive quote-reply in chronological order (D166).
    pub(super) async fn quote_thread(
        &mut self,
        conversation: ConversationId,
        target: MessageRef,
    ) -> Result<Vec<Message>> {
        let key = conversation_key(conversation);
        let sender_db = target.sender.to_string();
        let sent_at_db = timestamp_to_db(target.sent_at)?;
        let rows: Vec<MessageRow> = sqlx::query_as(
            "WITH RECURSIVE
             ancestors(sender, sent_at, depth) AS (
                 VALUES (?2, ?3, 0)
                 UNION
                 SELECT m.quote_sender, m.quote_sent_at, a.depth + 1
                 FROM messages m
                 JOIN ancestors a
                   ON m.sender = a.sender AND m.sent_at = a.sent_at
                 WHERE m.conversation_id = ?1
                   AND m.quote_sender IS NOT NULL
                   AND m.quote_sent_at IS NOT NULL
                   AND a.depth < 64
             ),
             root(sender, sent_at) AS (
                 SELECT sender, sent_at
                 FROM ancestors
                 ORDER BY depth DESC
                 LIMIT 1
             ),
             thread_refs(sender, sent_at, depth) AS (
                 SELECT sender, sent_at, 0 FROM root
                 UNION
                 SELECT m.sender, m.sent_at, t.depth + 1
                 FROM messages m
                 JOIN thread_refs t
                   ON m.quote_sender = t.sender
                  AND m.quote_sent_at = t.sent_at
                 WHERE m.conversation_id = ?1
                   AND t.depth < 64
             )
             SELECT id, sender, sent_at, body, quote_sender, quote_sent_at, quote_preview,
                    edited_at, deleted, direction, status, expires_in_secs, expire_started_at,
                    system_event
             FROM messages
             WHERE conversation_id = ?1
               AND (sender, sent_at) IN (SELECT sender, sent_at FROM thread_refs)
             ORDER BY sent_at ASC, sender ASC
             LIMIT 200",
        )
        .bind(&key)
        .bind(sender_db)
        .bind(sent_at_db)
        .fetch_all(&mut self.conn)
        .await?;

        let mut ids = Vec::with_capacity(rows.len());
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let (id, message) = decode_message_row(conversation, row)?;
            ids.push(id);
            messages.push(message);
        }
        self.attach_extras(&ids, &mut messages).await?;
        Ok(messages)
    }

    /// Search non-deleted messages matching `query` (free-text tokens across
    /// `body`, attachment `transcript`, and attachment `file_name`, plus
    /// `from:`, `in:`, `has:`, `is:`, `before:`, `after:` operators), ordered
    /// newest first (M12, #26).
    #[expect(
        clippy::too_many_lines,
        reason = "SQL query builder covers every search operator in one pass"
    )]
    pub(super) async fn search_messages(
        &mut self,
        conversation: Option<ConversationId>,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SearchHit>> {
        use super::search::{HasFilter, IsFilter, SearchQuery};

        type SearchRow = (
            String,
            i64,
            String,
            i64,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<i64>,
            bool,
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<String>,
        );
        let parsed = SearchQuery::parse(query);
        if parsed.is_empty() || limit == 0 {
            return Ok(vec![]);
        }
        let mut builder: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
            "SELECT m.conversation_id, m.id, m.sender, m.sent_at, m.body,
                    m.quote_sender, m.quote_sent_at, m.quote_preview,
                    m.edited_at, m.deleted, m.direction, m.status,
                    m.expires_in_secs, m.expire_started_at, m.system_event
             FROM messages m
             WHERE m.deleted = 0",
        );
        if let Some(conv) = conversation {
            builder.push(" AND m.conversation_id = ");
            builder.push_bind(conversation_key(conv));
        }
        for token in &parsed.terms {
            let pattern = like_pattern(token);
            builder.push(" AND (lower(coalesce(m.body, '')) LIKE ");
            builder.push_bind(pattern.clone());
            builder.push(
                " ESCAPE '\\' OR EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id AND (lower(coalesce(a.transcript, '')) LIKE ",
            );
            builder.push_bind(pattern.clone());
            builder.push(" ESCAPE '\\' OR lower(coalesce(a.file_name, '')) LIKE ");
            builder.push_bind(pattern.clone());
            builder.push(
                " ESCAPE '\\')) OR EXISTS (SELECT 1 FROM link_previews lp WHERE lp.message_id = m.id AND (lower(lp.url) LIKE ",
            );
            builder.push_bind(pattern.clone());
            builder.push(" ESCAPE '\\' OR lower(coalesce(lp.title, '')) LIKE ");
            builder.push_bind(pattern.clone());
            builder.push(" ESCAPE '\\' OR lower(coalesce(lp.description, '')) LIKE ");
            builder.push_bind(pattern);
            builder.push(" ESCAPE '\\')))");
        }
        for who in &parsed.from {
            if who == "me" || who == "you" {
                builder.push(" AND m.direction = 'outgoing'");
            } else {
                let pattern = like_pattern(who);
                builder.push(" AND (lower(m.sender) LIKE ");
                builder.push_bind(pattern.clone());
                builder.push(
                    " ESCAPE '\\' OR EXISTS (SELECT 1 FROM recipients r WHERE r.service_id = m.sender AND (lower(coalesce(r.nickname, '')) LIKE ",
                );
                builder.push_bind(pattern.clone());
                builder.push(" ESCAPE '\\' OR lower(coalesce(r.contact_name, '')) LIKE ");
                builder.push_bind(pattern.clone());
                builder.push(" ESCAPE '\\' OR lower(coalesce(r.profile_name, '')) LIKE ");
                builder.push_bind(pattern.clone());
                builder.push(" ESCAPE '\\' OR lower(coalesce(r.phone, '')) LIKE ");
                builder.push_bind(pattern);
                builder.push(" ESCAPE '\\')))");
            }
        }
        for chat in &parsed.in_chat {
            let pattern = like_pattern(chat);
            builder.push(
                " AND EXISTS (SELECT 1 FROM conversations c LEFT JOIN recipients r ON r.service_id = c.peer WHERE c.id = m.conversation_id AND (lower(coalesce(c.title, '')) LIKE ",
            );
            builder.push_bind(pattern.clone());
            builder.push(" ESCAPE '\\' OR lower(coalesce(r.nickname, '')) LIKE ");
            builder.push_bind(pattern.clone());
            builder.push(" ESCAPE '\\' OR lower(coalesce(r.contact_name, '')) LIKE ");
            builder.push_bind(pattern.clone());
            builder.push(" ESCAPE '\\' OR lower(coalesce(r.profile_name, '')) LIKE ");
            builder.push_bind(pattern.clone());
            builder.push(" ESCAPE '\\' OR lower(coalesce(r.phone, '')) LIKE ");
            builder.push_bind(pattern);
            builder.push(" ESCAPE '\\'))");
        }
        for filter in &parsed.has {
            let clause = match filter {
                HasFilter::Image => {
                    " AND EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id AND lower(a.content_type) LIKE 'image/%')"
                }
                HasFilter::Video => {
                    " AND EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id AND lower(a.content_type) LIKE 'video/%')"
                }
                HasFilter::Voice => {
                    " AND EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id AND (a.voice_note = 1 OR lower(a.content_type) LIKE 'audio/%'))"
                }
                HasFilter::File => {
                    " AND EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id)"
                }
                HasFilter::Link => {
                    " AND (lower(coalesce(m.body, '')) LIKE '%http://%' OR lower(coalesce(m.body, '')) LIKE '%https://%' OR EXISTS (SELECT 1 FROM link_previews lp WHERE lp.message_id = m.id))"
                }
                HasFilter::Transcript => {
                    " AND EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id AND coalesce(a.transcript, '') != '')"
                }
                HasFilter::Reaction => {
                    " AND EXISTS (SELECT 1 FROM reactions rx WHERE rx.message_id = m.id)"
                }
                HasFilter::Reply => " AND m.quote_sent_at IS NOT NULL",
            };
            builder.push(clause);
        }
        for filter in &parsed.is {
            let clause = match filter {
                IsFilter::Unread => {
                    " AND (m.direction = 'incoming' AND m.system_event IS NULL AND (SELECT count(*) FROM messages m2 WHERE m2.conversation_id = m.conversation_id AND m2.direction = 'incoming' AND m2.deleted = 0 AND m2.system_event IS NULL AND (m2.sent_at, m2.sender) >= (m.sent_at, m.sender)) <= (SELECT coalesce(c.unread_count, 0) FROM conversations c WHERE c.id = m.conversation_id))"
                }
                IsFilter::Read => {
                    " AND NOT (m.direction = 'incoming' AND m.system_event IS NULL AND (SELECT count(*) FROM messages m2 WHERE m2.conversation_id = m.conversation_id AND m2.direction = 'incoming' AND m2.deleted = 0 AND m2.system_event IS NULL AND (m2.sent_at, m2.sender) >= (m.sent_at, m.sender)) <= (SELECT coalesce(c.unread_count, 0) FROM conversations c WHERE c.id = m.conversation_id))"
                }
                IsFilter::Edited => " AND m.edited_at IS NOT NULL",
                IsFilter::Incoming => " AND m.direction = 'incoming'",
                IsFilter::Outgoing => " AND m.direction = 'outgoing'",
            };
            builder.push(clause);
        }
        if let Some(before_ms) = parsed.before_ms {
            builder.push(" AND m.sent_at < ");
            builder.push_bind(before_ms);
        }
        if let Some(after_ms) = parsed.after_ms {
            builder.push(" AND m.sent_at >= ");
            builder.push_bind(after_ms);
        }
        builder.push(" ORDER BY m.sent_at DESC, m.sender ASC LIMIT ");
        builder.push_bind(i64::from(limit));

        let rows: Vec<SearchRow> = builder.build_query_as().fetch_all(&mut self.conn).await?;
        let mut ids = Vec::with_capacity(rows.len());
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let conv = conversation_from_key(&row.0)?;
            let msg_row = (
                row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, row.10, row.11,
                row.12, row.13, row.14,
            );
            let (id, message) = decode_message_row(conv, msg_row)?;
            ids.push(id);
            messages.push(message);
        }
        self.attach_extras(&ids, &mut messages).await?;
        Ok(messages
            .into_iter()
            .map(|message| {
                let matched_transcript = message.attachments.iter().find_map(|att| {
                    let text = att.transcript.as_ref()?;
                    let lower = text.to_lowercase();
                    if parsed.terms.is_empty() {
                        parsed
                            .has
                            .iter()
                            .any(|h| matches!(h, HasFilter::Voice | HasFilter::Transcript))
                            .then(|| text.clone())
                    } else {
                        parsed
                            .terms
                            .iter()
                            .any(|token| lower.contains(token))
                            .then(|| text.clone())
                    }
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
    pub(super) async fn message_receipts(
        &mut self,
        message: MessageRef,
    ) -> Result<Vec<RecipientReceipt>> {
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT recipient, status, updated_at
             FROM message_receipts
             WHERE sender = ?1 AND sent_at = ?2
             ORDER BY updated_at ASC, recipient ASC",
        )
        .bind(message.sender.to_string())
        .bind(timestamp_to_db(message.sent_at)?)
        .fetch_all(&mut self.conn)
        .await?;
        rows.into_iter()
            .map(|(recipient, status, updated_at)| {
                Ok(RecipientReceipt {
                    recipient: service_id_from_db(&recipient, "message_receipts.recipient")?,
                    status: status_from_db(&status)?,
                    updated_at: timestamp_from_db(updated_at, "message_receipts.updated_at")?,
                })
            })
            .collect()
    }

    /// Fill in reactions, attachments and mentions for a slice of messages
    /// (`ids[i]` is the row of `messages[i]`).
    async fn attach_extras(&mut self, ids: &[i64], messages: &mut [Message]) -> Result<()> {
        let (Some(&first), Some(&last)) = (ids.first(), ids.last()) else {
            return Ok(());
        };
        let index: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let (low, high) = ids
            .iter()
            .fold((first, last), |(lo, hi), &id| (lo.min(id), hi.max(id)));
        // Row IDs of a batch aren't contiguous, but filtering by the row-ID
        // range and then by `index` avoids a variable-length IN list.
        let reactions: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT r.message_id, r.sender, r.emoji FROM reactions r
             WHERE r.message_id BETWEEN ?1 AND ?2
             ORDER BY r.rowid",
        )
        .bind(low)
        .bind(high)
        .fetch_all(&mut self.conn)
        .await?;
        for (id, sender, emoji) in reactions {
            if let Some(message) = index.get(&id).and_then(|&i| messages.get_mut(i)) {
                let sender = service_id_from_db(&sender, "reaction sender")?;
                message.reactions.push(Reaction { sender, emoji });
            }
        }

        attachments::load(&mut self.conn, low, high, &index, messages).await?;

        let mentions: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "SELECT n.message_id, n.start, n.length, n.service_id
             FROM mentions n
             WHERE n.message_id BETWEEN ?1 AND ?2
             ORDER BY n.message_id, n.start",
        )
        .bind(low)
        .bind(high)
        .fetch_all(&mut self.conn)
        .await?;
        for (id, start, len, who) in mentions {
            if let Some(message) = index.get(&id).and_then(|&i| messages.get_mut(i)) {
                let offset =
                    |n: i64| usize::try_from(n).map_err(|_| StoreError::Corrupt("mention"));
                message.mentions.push(Mention {
                    who: service_id_from_db(&who, "mention")?,
                    start: offset(start)?,
                    len: offset(len)?,
                });
            }
        }

        let previews: Vec<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT lp.message_id, lp.url, lp.title, lp.description
             FROM link_previews lp
             WHERE lp.message_id BETWEEN ?1 AND ?2
             ORDER BY lp.message_id, lp.idx",
        )
        .bind(low)
        .bind(high)
        .fetch_all(&mut self.conn)
        .await?;
        for (id, url, title, description) in previews {
            if let Some(message) = index.get(&id).and_then(|&i| messages.get_mut(i)) {
                message.previews.push(LinkPreview {
                    url,
                    title,
                    description,
                });
            }
        }
        Ok(())
    }

    pub(super) async fn set_conversation_state(
        &mut self,
        conversation: ConversationId,
        state: &ConversationState,
    ) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        let key = ensure_conversation(&mut tx, conversation).await?;
        sqlx::query(
            "INSERT INTO conversation_state
                 (conversation_id, hidden, archived, pinned, muted_until, draft, allowed, notify,
                  remote_transcription, message_request, blocked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT (conversation_id) DO UPDATE SET
                 hidden = excluded.hidden, archived = excluded.archived,
                 pinned = excluded.pinned, muted_until = excluded.muted_until,
                 draft = excluded.draft, allowed = excluded.allowed,
                 notify = excluded.notify,
                 remote_transcription = excluded.remote_transcription,
                 message_request = excluded.message_request,
                 blocked = excluded.blocked",
        )
        .bind(&key)
        .bind(state.hidden)
        .bind(state.archived)
        .bind(state.pinned.map(i64::from))
        .bind(state.muted_until.map(timestamp_to_db).transpose()?)
        .bind(state.draft.as_ref().map(MessageBody::expose))
        .bind(state.allowed)
        .bind(state.notify.as_str())
        .bind(state.remote_transcription.as_bool())
        .bind(state.request)
        .bind(state.blocked)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn mark_read(&mut self, conversation: ConversationId) -> Result<()> {
        let key = conversation_key(conversation);
        let now_db = timestamp_to_db(now())?;
        let mut tx = self.conn.begin().await?;
        sqlx::query("UPDATE conversations SET unread_count = 0 WHERE id = ?1")
            .bind(&key)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE messages SET expire_started_at = ?1
             WHERE conversation_id = ?2
               AND expires_in_secs IS NOT NULL
               AND expire_started_at IS NULL",
        )
        .bind(now_db)
        .bind(&key)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Mark outgoing messages still `sending` as `failed`; returns how many.
    pub(super) async fn fail_unsent(&mut self) -> Result<u64> {
        let done = sqlx::query(
            "UPDATE messages SET status = ?1 WHERE direction = 'outgoing' AND status = ?2",
        )
        .bind(status_to_db(DeliveryStatus::Failed))
        .bind(status_to_db(DeliveryStatus::Sending))
        .execute(&mut self.conn)
        .await?;
        Ok(done.rows_affected())
    }

    /// See [`Store::claim_profile`](super::Store::claim_profile). Reading
    /// and marking happen in one transaction.
    pub(super) async fn claim_profile(&mut self, wanted: Profile) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        let stored = stored_profile(&mut tx).await?;
        if decide(wanted, stored)? {
            sqlx::query("INSERT INTO app_state (key, value) VALUES (?1, ?2)")
                .bind(PROFILE_KEY)
                .bind(wanted.as_str())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn app_state(&mut self, key: &str) -> Result<Option<String>> {
        let value: Option<(String,)> = sqlx::query_as("SELECT value FROM app_state WHERE key = ?1")
            .bind(key)
            .fetch_optional(&mut self.conn)
            .await?;
        Ok(value.map(|(v,)| v))
    }

    pub(super) async fn set_app_state(&mut self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_state (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&mut self.conn)
        .await?;
        Ok(())
    }

    pub(super) async fn remove_app_state(&mut self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM app_state WHERE key = ?1")
            .bind(key)
            .execute(&mut self.conn)
            .await?;
        Ok(())
    }

    /// Everyone the user gave a nickname (D65).
    pub(super) async fn nicknames(&mut self) -> Result<HashMap<ServiceId, String>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT service_id, nickname FROM recipients WHERE nickname IS NOT NULL",
        )
        .fetch_all(&mut self.conn)
        .await?;
        rows.into_iter()
            .map(|(id, nickname)| Ok((service_id_from_db(&id, "recipients.service_id")?, nickname)))
            .collect()
    }

    /// Give someone a nickname, or take it away (`None`).
    pub(super) async fn set_nickname(
        &mut self,
        id: ServiceId,
        nickname: Option<&str>,
    ) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        ensure_recipient(&mut tx, id).await?;
        sqlx::query("UPDATE recipients SET nickname = ?1 WHERE service_id = ?2")
            .bind(nickname)
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Everyone whose safety number or non-default verification state is recorded (D163).
    pub(super) async fn identities(&mut self) -> Result<HashMap<ServiceId, IdentityRecord>> {
        let rows: Vec<(String, Option<String>, String, bool)> = sqlx::query_as(
            "SELECT service_id, safety_number, verified, key_changed FROM recipients
             WHERE safety_number IS NOT NULL OR verified != 'default' OR key_changed != 0",
        )
        .fetch_all(&mut self.conn)
        .await?;
        rows.into_iter()
            .map(|(id, safety_number, verified, key_changed)| {
                Ok((
                    service_id_from_db(&id, "recipients.service_id")?,
                    IdentityRecord {
                        safety_number,
                        verified: verification_from_db(&verified)?,
                        key_changed,
                    },
                ))
            })
            .collect()
    }

    /// Record safety numbers and optional synced verification states from the backend (D163).
    pub(super) async fn upsert_identities(&mut self, updates: &[IdentityUpdate]) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        for update in updates {
            ensure_recipient(&mut tx, update.service_id).await?;
            let id_str = update.service_id.to_string();
            let (safety_number, verified, key_changed): (Option<String>, String, bool) =
                sqlx::query_as(
                    "SELECT safety_number, verified, key_changed FROM recipients
                     WHERE service_id = ?1",
                )
                .bind(&id_str)
                .fetch_one(&mut *tx)
                .await?;
            let mut record = IdentityRecord {
                safety_number,
                verified: verification_from_db(&verified)?,
                key_changed,
            };
            record.update(update.safety_number.clone(), update.verified);
            sqlx::query(
                "UPDATE recipients
                 SET safety_number = ?1, verified = ?2, key_changed = ?3
                 WHERE service_id = ?4",
            )
            .bind(record.safety_number.as_deref())
            .bind(record.verified.as_str())
            .bind(i64::from(record.key_changed))
            .bind(&id_str)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Record a user-chosen verification state (or approval of a changed key),
    /// clearing `key_changed` (D163).
    pub(super) async fn set_verification(
        &mut self,
        id: ServiceId,
        verified: VerificationState,
    ) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        ensure_recipient(&mut tx, id).await?;
        sqlx::query("UPDATE recipients SET verified = ?1, key_changed = 0 WHERE service_id = ?2")
            .bind(verified.as_str())
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn upsert_disappearing_timers(
        &mut self,
        timers: &[(ConversationId, u32)],
    ) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        for &(conversation, seconds) in timers {
            let key = ensure_conversation(&mut tx, conversation).await?;
            let timer = (seconds > 0).then(|| i64::from(seconds));
            sqlx::query("UPDATE conversations SET disappearing_timer = ?1 WHERE id = ?2")
                .bind(timer)
                .bind(&key)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn set_disappearing_timer(
        &mut self,
        conversation: ConversationId,
        seconds: Option<u32>,
    ) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        let key = ensure_conversation(&mut tx, conversation).await?;
        let timer = seconds.filter(|&s| s > 0).map(i64::from);
        sqlx::query("UPDATE conversations SET disappearing_timer = ?1 WHERE id = ?2")
            .bind(timer)
            .bind(&key)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn delete_conversation(
        &mut self,
        conversation: ConversationId,
    ) -> Result<Vec<PathBuf>> {
        let key = conversation_key(conversation);
        let mut tx = self.conn.begin().await?;
        let rows: Vec<(i64, String, i64)> = sqlx::query_as(
            "SELECT id, sender, sent_at FROM messages WHERE conversation_id = ?1 ORDER BY id",
        )
        .bind(&key)
        .fetch_all(&mut *tx)
        .await?;
        let mut attachment_paths = Vec::new();
        for (id, sender, sent_at) in rows {
            let paths: Vec<(String,)> = sqlx::query_as(
                "SELECT local_path FROM attachments
                 WHERE message_id = ?1 AND status = 'downloaded' AND local_path IS NOT NULL
                 ORDER BY id",
            )
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
            for (path,) in paths {
                attachment_paths.push(PathBuf::from(path));
            }
            for table in [
                "DELETE FROM reactions WHERE message_id = ?1",
                "DELETE FROM attachments WHERE message_id = ?1",
                "DELETE FROM mentions WHERE message_id = ?1",
                "DELETE FROM messages WHERE id = ?1",
            ] {
                sqlx::query(table).bind(id).execute(&mut *tx).await?;
            }
            sqlx::query("DELETE FROM message_receipts WHERE sender = ?1 AND sent_at = ?2")
                .bind(sender)
                .bind(sent_at)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "UPDATE conversations SET last_activity = NULL, unread_count = 0 WHERE id = ?1",
        )
        .bind(&key)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE conversation_state SET message_request = 0, draft = NULL
             WHERE conversation_id = ?1",
        )
        .bind(&key)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(attachment_paths)
    }

    async fn sync_blocked(&mut self, conversations: &[ConversationId]) -> Result<()> {
        let mut tx = self.conn.begin().await?;
        let mut blocked_keys = HashSet::with_capacity(conversations.len());
        for &conv in conversations {
            let key = ensure_conversation(&mut tx, conv).await?;
            blocked_keys.insert(key.clone());
            sqlx::query(
                "INSERT INTO conversation_state (conversation_id, blocked, message_request)
                 VALUES (?1, 1, 0)
                 ON CONFLICT (conversation_id) DO UPDATE SET
                     blocked = 1, message_request = 0",
            )
            .bind(&key)
            .execute(&mut *tx)
            .await?;
        }
        let currently_blocked: Vec<(String,)> =
            sqlx::query_as("SELECT conversation_id FROM conversation_state WHERE blocked != 0")
                .fetch_all(&mut *tx)
                .await?;
        for (key,) in currently_blocked {
            if !blocked_keys.contains(&key) {
                sqlx::query("UPDATE conversation_state SET blocked = 0 WHERE conversation_id = ?1")
                    .bind(&key)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    async fn apply_message_request_response(
        &mut self,
        conversation: ConversationId,
        action: MessageRequestAction,
    ) -> Result<()> {
        match action {
            MessageRequestAction::Accept => {
                let mut tx = self.conn.begin().await?;
                let key = ensure_conversation(&mut tx, conversation).await?;
                sqlx::query(
                    "INSERT INTO conversation_state (conversation_id, message_request, blocked)
                     VALUES (?1, 0, 0)
                     ON CONFLICT (conversation_id) DO UPDATE SET
                         message_request = 0, blocked = 0",
                )
                .bind(&key)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
            }
            MessageRequestAction::Delete | MessageRequestAction::Spam => {
                self.delete_conversation(conversation).await?;
            }
            MessageRequestAction::Block => {
                let mut tx = self.conn.begin().await?;
                let key = ensure_conversation(&mut tx, conversation).await?;
                sqlx::query(
                    "INSERT INTO conversation_state (conversation_id, message_request, blocked)
                     VALUES (?1, 0, 1)
                     ON CONFLICT (conversation_id) DO UPDATE SET
                         message_request = 0, blocked = 1",
                )
                .bind(&key)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
            }
            MessageRequestAction::BlockAndDelete | MessageRequestAction::BlockAndSpam => {
                self.delete_conversation(conversation).await?;
                let mut tx = self.conn.begin().await?;
                let key = ensure_conversation(&mut tx, conversation).await?;
                sqlx::query(
                    "INSERT INTO conversation_state (conversation_id, message_request, blocked)
                     VALUES (?1, 0, 1)
                     ON CONFLICT (conversation_id) DO UPDATE SET
                         message_request = 0, blocked = 1",
                )
                .bind(&key)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
            }
            MessageRequestAction::Unblock => {
                let mut tx = self.conn.begin().await?;
                let key = ensure_conversation(&mut tx, conversation).await?;
                sqlx::query("UPDATE conversation_state SET blocked = 0 WHERE conversation_id = ?1")
                    .bind(&key)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
        }
        Ok(())
    }

    pub(super) async fn sweep_expired(&mut self, now: Timestamp) -> Result<ExpiredSweep> {
        let now_db = timestamp_to_db(now)?;
        let mut tx = self.conn.begin().await?;
        let expired: Vec<(i64, String, String, i64)> = sqlx::query_as(
            "SELECT id, conversation_id, sender, sent_at
             FROM messages
             WHERE expires_in_secs IS NOT NULL
               AND expire_started_at IS NOT NULL
               AND expire_started_at + expires_in_secs * 1000 <= ?1
             ORDER BY sent_at ASC, sender ASC",
        )
        .bind(now_db)
        .fetch_all(&mut *tx)
        .await?;
        if expired.is_empty() {
            return Ok(ExpiredSweep::default());
        }
        let mut messages = Vec::with_capacity(expired.len());
        let mut attachment_paths = Vec::new();
        let affected_keys: std::collections::BTreeSet<String> =
            expired.iter().map(|(_, key, _, _)| key.clone()).collect();
        let mut remaining_unreads = HashMap::with_capacity(affected_keys.len());
        for key in &affected_keys {
            let (surviving_unread,): (i64,) = sqlx::query_as(
                "SELECT count(*) FROM (
                     SELECT expires_in_secs, expire_started_at FROM messages
                     WHERE conversation_id = ?1 AND direction = 'incoming' AND deleted = 0
                       AND system_event IS NULL
                     ORDER BY sent_at DESC, sender DESC
                     LIMIT (SELECT unread_count FROM conversations WHERE id = ?1)
                 ) WHERE NOT (
                     expires_in_secs IS NOT NULL
                     AND expire_started_at IS NOT NULL
                     AND expire_started_at + expires_in_secs * 1000 <= ?2
                 )",
            )
            .bind(key)
            .bind(now_db)
            .fetch_one(&mut *tx)
            .await?;
            remaining_unreads.insert(key.clone(), surviving_unread);
        }
        for (id, key, sender, sent_at) in expired {
            let conv = conversation_from_key(&key)?;
            let msg_ref = MessageRef {
                sender: service_id_from_db(&sender, "sender")?,
                sent_at: timestamp_from_db(sent_at, "sent_at")?,
            };
            let paths: Vec<(String,)> = sqlx::query_as(
                "SELECT local_path FROM attachments
                 WHERE message_id = ?1 AND status = 'downloaded' AND local_path IS NOT NULL
                 ORDER BY id",
            )
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
            for (path,) in paths {
                attachment_paths.push(std::path::PathBuf::from(path));
            }
            for table in [
                "DELETE FROM reactions WHERE message_id = ?1",
                "DELETE FROM attachments WHERE message_id = ?1",
                "DELETE FROM mentions WHERE message_id = ?1",
                "DELETE FROM messages WHERE id = ?1",
            ] {
                sqlx::query(table).bind(id).execute(&mut *tx).await?;
            }
            sqlx::query("DELETE FROM message_receipts WHERE sender = ?1 AND sent_at = ?2")
                .bind(&sender)
                .bind(sent_at)
                .execute(&mut *tx)
                .await?;
            messages.push((conv, msg_ref));
        }
        for key in &affected_keys {
            let unread = remaining_unreads.get(key).copied().unwrap_or(0);
            sqlx::query(
                "UPDATE conversations
                 SET last_activity = (SELECT max(sent_at) FROM messages WHERE conversation_id = ?1),
                     unread_count = ?2
                 WHERE id = ?1",
            )
            .bind(key)
            .bind(unread)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        let mut conversations = Vec::with_capacity(affected_keys.len());
        for key in affected_keys {
            conversations.push(self.swept_conversation_summary(&key).await?);
        }
        conversations.sort_by_key(|a| a.0);
        Ok(ExpiredSweep {
            messages,
            attachment_paths,
            conversations,
        })
    }

    async fn swept_conversation_summary(
        &mut self,
        key: &str,
    ) -> Result<(ConversationId, Option<Timestamp>, Option<Message>, u32)> {
        let conv = conversation_from_key(key)?;
        let (last_activity, unread): (Option<i64>, i64) =
            sqlx::query_as("SELECT last_activity, unread_count FROM conversations WHERE id = ?1")
                .bind(key)
                .fetch_one(&mut self.conn)
                .await?;
        let newest: Option<MessageRow> = sqlx::query_as(
            "SELECT id, sender, sent_at, body, quote_sender, quote_sent_at, quote_preview,
                    edited_at, deleted, direction, status, expires_in_secs, expire_started_at,
                    system_event
             FROM messages
             WHERE conversation_id = ?1
             ORDER BY sent_at DESC, sender DESC
             LIMIT 1",
        )
        .bind(key)
        .fetch_optional(&mut self.conn)
        .await?;
        let last_message = if let Some(row) = newest {
            let (id, msg) = decode_message_row(conv, row)?;
            let mut one = [msg];
            self.attach_extras(&[id], &mut one).await?;
            let [msg] = one;
            Some(msg)
        } else {
            None
        };
        Ok((
            conv,
            optional_timestamp_from_db(last_activity, "last_activity")?,
            last_message,
            count_from_db(unread, "unread_count")?,
        ))
    }

    pub(super) async fn next_expiry(&mut self) -> Result<Option<Timestamp>> {
        let row: (Option<i64>,) = sqlx::query_as(
            "SELECT min(expire_started_at + expires_in_secs * 1000)
             FROM messages
             WHERE expires_in_secs IS NOT NULL AND expire_started_at IS NOT NULL",
        )
        .fetch_one(&mut self.conn)
        .await?;
        optional_timestamp_from_db(row.0, "next_expiry")
    }

    pub(super) async fn stats(&mut self) -> Result<StoreStats> {
        let (conversations, messages): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM conversations), (SELECT count(*) FROM messages)",
        )
        .fetch_one(&mut self.conn)
        .await?;
        Ok(StoreStats {
            conversations: u64::try_from(conversations)
                .map_err(|_| StoreError::Corrupt("count"))?,
            messages: u64::try_from(messages).map_err(|_| StoreError::Corrupt("count"))?,
        })
    }
}

/// Read the profile without changing anything; works before migrations.
async fn stored_profile(conn: &mut SqliteConnection) -> Result<StoredProfile> {
    let tables: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table'")
            .fetch_all(&mut *conn)
            .await?;
    if tables.is_empty() {
        return Ok(StoredProfile::New);
    }
    if !tables.iter().any(|(name,)| name == "app_state") {
        // Some other SQLite file, or one faro never finished creating.
        return Ok(StoredProfile::Unmarked { empty: false });
    }
    let stored: Option<(String,)> = sqlx::query_as("SELECT value FROM app_state WHERE key = ?1")
        .bind(PROFILE_KEY)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some((text,)) = stored {
        return Ok(StoredProfile::Marked(text));
    }
    // `app_state` came with the first migration, as did these; every other
    // table hangs off them.
    let (rows,): (i64,) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM app_state) + (SELECT count(*) FROM recipients)
              + (SELECT count(*) FROM conversations) + (SELECT count(*) FROM messages)",
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(StoredProfile::Unmarked { empty: rows == 0 })
}

/// Read the profile of the database file at `path` leaving no trace: read
/// only, and with no `-wal` file around (everything is in the main file)
/// also immutable, so SQLite doesn't create `-wal`/`-shm` files beside it.
async fn peek_profile(path: &Path) -> Result<StoredProfile> {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let wal = std::path::PathBuf::from(wal);
    let wal_exists = tokio::task::spawn_blocking(move || crate::paths::exists(&wal))
        .await
        .map_err(|e| StoreError::Io(std::io::Error::other(e)))??;
    let mut options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .immutable(!wal_exists);
    if let Some(pragma) = crate::vault::faro_db_pragma_for(path) {
        options = options.pragma("key", pragma);
    }
    let mut conn = options.connect().await?;
    let stored = stored_profile(&mut conn).await;
    conn.close().await?;
    stored
}

/// The profile the database at `path` is marked with, read without
/// changing it; `None` if it isn't marked (or with one unknown here).
pub(super) async fn profile_of(path: &Path) -> Result<Option<Profile>> {
    Ok(match peek_profile(path).await? {
        StoredProfile::Marked(text) => Profile::parse(&text),
        StoredProfile::New | StoredProfile::Unmarked { .. } => None,
    })
}

/// Make sure a `recipients` row exists (names unknown until a contact update).
async fn ensure_recipient(conn: &mut SqliteConnection, id: ServiceId) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO recipients (service_id) VALUES (?1)")
        .bind(id.to_string())
        .execute(conn)
        .await?;
    Ok(())
}

/// Make sure a `conversations` row exists; returns its key.
async fn ensure_conversation(conn: &mut SqliteConnection, id: ConversationId) -> Result<String> {
    let key = conversation_key(id);
    match id {
        ConversationId::Direct(peer) => {
            ensure_recipient(&mut *conn, peer).await?;
            sqlx::query(
                "INSERT OR IGNORE INTO conversations (id, kind, peer) VALUES (?1, 'direct', ?2)",
            )
            .bind(&key)
            .bind(peer.to_string())
            .execute(conn)
            .await?;
        }
        ConversationId::Group(_) => {
            sqlx::query("INSERT OR IGNORE INTO conversations (id, kind) VALUES (?1, 'group')")
                .bind(&key)
                .execute(conn)
                .await?;
        }
    }
    Ok(key)
}

async fn message_id(conn: &mut SqliteConnection, target: MessageRef) -> Result<Option<i64>> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM messages WHERE sender = ?1 AND sent_at = ?2")
            .bind(target.sender.to_string())
            .bind(timestamp_to_db(target.sent_at)?)
            .fetch_optional(conn)
            .await?;
    Ok(row.map(|(id,)| id))
}

/// One reaction per sender: a new one replaces the old.
async fn insert_reaction(
    conn: &mut SqliteConnection,
    message: i64,
    sender: ServiceId,
    emoji: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO reactions (message_id, sender, emoji) VALUES (?1, ?2, ?3)
         ON CONFLICT (message_id, sender) DO UPDATE SET emoji = excluded.emoji",
    )
    .bind(message)
    .bind(sender.to_string())
    .bind(emoji)
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_mentions(
    conn: &mut SqliteConnection,
    message: i64,
    mentions: &[Mention],
) -> Result<()> {
    for mention in mentions {
        let offset = |n: usize| i64::try_from(n).map_err(|_| StoreError::OutOfRange("mention"));
        sqlx::query(
            "INSERT INTO mentions (message_id, start, length, service_id) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (message_id, start) DO NOTHING",
        )
        .bind(message)
        .bind(offset(mention.start)?)
        .bind(offset(mention.len)?)
        .bind(mention.who.to_string())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn insert_link_previews(
    conn: &mut SqliteConnection,
    message: i64,
    previews: &[LinkPreview],
) -> Result<()> {
    for (idx, preview) in previews.iter().enumerate() {
        let idx = i64::try_from(idx).map_err(|_| StoreError::OutOfRange("link_preview"))?;
        sqlx::query(
            "INSERT INTO link_previews (message_id, idx, url, title, description)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (message_id, idx) DO NOTHING",
        )
        .bind(message)
        .bind(idx)
        .bind(&preview.url)
        .bind(preview.title.as_deref())
        .bind(preview.description.as_deref())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

fn decode_message_row(conversation: ConversationId, row: MessageRow) -> Result<(i64, Message)> {
    let (id, sender, sent_at, body, quote_sender, quote_sent_at, quote_preview) =
        (row.0, row.1, row.2, row.3, row.4, row.5, row.6);
    let (
        edited_at,
        deleted,
        direction,
        status,
        expires_in_secs,
        expire_started_at,
        system_event_raw,
    ) = (row.7, row.8, row.9, row.10, row.11, row.12, row.13);
    let quote = match (quote_sender, quote_sent_at) {
        (Some(sender), Some(sent_at)) => Some(Quote {
            target: MessageRef {
                sender: service_id_from_db(&sender, "quote_sender")?,
                sent_at: timestamp_from_db(sent_at, "quote_sent_at")?,
            },
            preview: quote_preview.map(MessageBody::new),
        }),
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => return Err(StoreError::Corrupt("quote")),
    };
    let system_event = system_event_raw
        .as_deref()
        .map(system_event_from_db)
        .transpose()?;
    let message = Message {
        conversation,
        sender: service_id_from_db(&sender, "sender")?,
        sent_at: timestamp_from_db(sent_at, "sent_at")?,
        direction: direction_from_db(&direction, status.as_deref())?,
        body: body.map(MessageBody::new),
        mentions: vec![],
        quote,
        previews: vec![],
        attachments: vec![],
        reactions: vec![],
        edited_at: optional_timestamp_from_db(edited_at, "edited_at")?,
        deleted,
        expires_in_secs: expires_in_secs
            .map(|s| count_from_db(s, "expires_in_secs"))
            .transpose()?,
        expire_started_at: optional_timestamp_from_db(expire_started_at, "expire_started_at")?,
        system_event,
    };
    Ok((id, message))
}

fn like_pattern(token: &str) -> String {
    let mut out = String::with_capacity(token.len() + 2);
    out.push('%');
    for ch in token.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

fn now() -> Timestamp {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    Timestamp::from_millis(ms)
}

async fn coalesce_system_event(
    tx: &mut SqliteConnection,
    conversation: &str,
    message: &Message,
    sent_at: i64,
) -> Result<bool> {
    if let Some(SystemEvent::Call { state, .. }) = &message.system_event
        && *state != CallState::Incoming
    {
        let existing: Option<(i64, String)> = sqlx::query_as(
            "SELECT id, system_event FROM messages
             WHERE conversation_id = ?1
               AND system_event IN ('call:voice:incoming', 'call:video:incoming', 'call:group:incoming')
               AND abs(sent_at - ?2) <= 300000
             ORDER BY sent_at DESC, id DESC
             LIMIT 1",
        )
        .bind(conversation)
        .bind(sent_at)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((existing_id, existing_sys)) = existing {
            let kind =
                if let Ok(SystemEvent::Call { kind, .. }) = system_event_from_db(&existing_sys) {
                    kind
                } else {
                    crate::domain::CallKind::Voice
                };
            let updated = system_event_to_db(&SystemEvent::Call {
                kind,
                state: *state,
            });
            sqlx::query("UPDATE messages SET system_event = ?1 WHERE id = ?2")
                .bind(&updated)
                .bind(existing_id)
                .execute(&mut *tx)
                .await?;
            return Ok(true);
        }
    }
    if let Some(event @ SystemEvent::DisappearingTimer { .. }) = &message.system_event {
        let sys_str = system_event_to_db(event);
        let dup: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM messages
             WHERE conversation_id = ?1
               AND sender = ?2
               AND system_event = ?3
               AND abs(sent_at - ?4) <= 10000
             LIMIT 1",
        )
        .bind(conversation)
        .bind(message.sender.to_string())
        .bind(&sys_str)
        .bind(sent_at)
        .fetch_optional(&mut *tx)
        .await?;
        if dup.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn refresh_duplicate_message(
    tx: &mut SqliteConnection,
    message: &Message,
    sent_at: i64,
    expires_in_secs: Option<u32>,
    expire_started_at_db: Option<i64>,
) -> Result<()> {
    if expires_in_secs.is_some() {
        sqlx::query(
            "UPDATE messages
             SET expires_in_secs = coalesce(expires_in_secs, ?1),
                 expire_started_at = coalesce(expire_started_at, ?2)
             WHERE sender = ?3 AND sent_at = ?4",
        )
        .bind(expires_in_secs.map(i64::from))
        .bind(expire_started_at_db)
        .bind(message.sender.to_string())
        .bind(sent_at)
        .execute(&mut *tx)
        .await?;
    }
    attachments::refresh(tx, message.reference(), &message.attachments).await
}
