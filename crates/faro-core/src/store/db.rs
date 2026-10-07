//! SQL for the store. Owned by the store task (`store.rs`); nothing else touches
//! the connection (D22).

use std::collections::HashMap;

use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{ConnectOptions as _, Connection as _};

use super::codec::{
    conversation_from_key, conversation_key, count_from_db, direction_from_db, direction_to_db,
    optional_timestamp_from_db, service_id_from_db, status_from_db, status_to_db,
    timestamp_from_db, timestamp_to_db,
};
use super::{ConversationState, ConversationSummary, Result, StoreError, StoreStats};
use crate::backend::BackendEvent;
use crate::domain::{
    AttachmentId, AttachmentRef, Contact, ConversationId, DeliveryStatus, Direction, Group,
    Message, MessageBody, MessageRef, PhoneNumber, Quote, Reaction, ServiceId, Timestamp,
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// `(message_id, backend_id, content_type, file_name, size)`.
type AttachmentRow = (i64, String, String, Option<String>, Option<i64>);

pub(super) struct Db {
    conn: SqliteConnection,
}

impl Db {
    /// Connect and bring the schema up to date.
    pub(super) async fn open(options: &SqliteConnectOptions) -> Result<Self> {
        let mut conn = options.connect().await?;
        MIGRATOR.run(&mut conn).await?;
        Ok(Self { conn })
    }

    pub(super) async fn close(self) -> Result<()> {
        self.conn.close().await.map_err(StoreError::from)
    }

    /// Persist what a backend reported. Transient events (connection, typing,
    /// errors) aren't stored.
    pub(super) async fn apply(&mut self, event: BackendEvent) -> Result<()> {
        match event {
            BackendEvent::Message(message) => self.insert_message(&message).await,
            BackendEvent::DeliveryUpdate { message, status } => {
                self.update_status(message, status).await
            }
            BackendEvent::Reaction {
                target,
                sender,
                emoji,
                ..
            } => self.set_reaction(target, sender, emoji.as_deref()).await,
            BackendEvent::Edited {
                target,
                body,
                edited_at,
                ..
            } => self.edit(target, &body, edited_at).await,
            BackendEvent::Deleted { target, .. } => self.delete(target).await,
            BackendEvent::ContactUpdated(contact) => self.upsert_contacts(&[contact]).await,
            BackendEvent::GroupUpdated(group) => self.upsert_groups(&[group]).await,
            BackendEvent::Connection(_) | BackendEvent::Typing { .. } | BackendEvent::Error(_) => {
                Ok(())
            }
        }
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
                     profile_name = excluded.profile_name,
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

        let inserted = sqlx::query(
            "INSERT INTO messages (conversation_id, sender, sent_at, received_at, body,
                 quote_sender, quote_sent_at, quote_preview, edited_at, deleted, direction, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
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
        .execute(&mut *tx)
        .await?;

        // Already stored (a duplicate delivery): keep the first copy.
        if inserted.rows_affected() == 0 {
            return Ok(());
        }
        let id = inserted.last_insert_rowid();
        for reaction in &message.reactions {
            insert_reaction(&mut tx, id, reaction.sender, &reaction.emoji).await?;
        }
        for attachment in &message.attachments {
            insert_attachment(&mut tx, id, attachment).await?;
        }
        let unread = i64::from(message.direction == Direction::Incoming && !message.deleted);
        sqlx::query(
            "UPDATE conversations
             SET last_activity = max(coalesce(last_activity, ?1), ?1),
                 unread_count = unread_count + ?2
             WHERE id = ?3",
        )
        .bind(sent_at)
        .bind(unread)
        .bind(&conversation)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Receipts arrive late, twice or out of order: never move a status backwards.
    async fn update_status(&mut self, message: MessageRef, status: DeliveryStatus) -> Result<()> {
        let row: Option<(i64, Option<String>)> = sqlx::query_as(
            "SELECT id, status FROM messages
             WHERE sender = ?1 AND sent_at = ?2 AND direction = 'outgoing'",
        )
        .bind(message.sender.to_string())
        .bind(timestamp_to_db(message.sent_at)?)
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

    /// Edits can arrive out of order; only a newer edit replaces the text.
    async fn edit(
        &mut self,
        target: MessageRef,
        body: &MessageBody,
        edited_at: Timestamp,
    ) -> Result<()> {
        let edited_at = timestamp_to_db(edited_at)?;
        sqlx::query(
            "UPDATE messages SET body = ?1, edited_at = ?2
             WHERE sender = ?3 AND sent_at = ?4 AND deleted = 0
               AND (edited_at IS NULL OR edited_at < ?2)",
        )
        .bind(body.expose())
        .bind(edited_at)
        .bind(target.sender.to_string())
        .bind(timestamp_to_db(target.sent_at)?)
        .execute(&mut self.conn)
        .await?;
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
        ] {
            sqlx::query(table).bind(id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Every conversation: pinned first (in pin order), then most recent activity,
    /// then empty ones. Filtering (hidden, archived, empty) is the UI's job.
    pub(super) async fn conversations(&mut self) -> Result<Vec<ConversationSummary>> {
        type Row = (
            String,
            Option<String>,
            Option<i64>,
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<bool>,
            Option<bool>,
            Option<i64>,
            Option<i64>,
            Option<String>,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT c.id, c.title, c.last_activity, c.unread_count,
                    r.contact_name, r.profile_name, r.phone,
                    s.hidden, s.archived, s.pinned, s.muted_until, s.draft
             FROM conversations c
             LEFT JOIN recipients r ON r.service_id = c.peer
             LEFT JOIN conversation_state s ON s.conversation_id = c.id
             ORDER BY s.pinned IS NULL, s.pinned,
                      c.last_activity IS NULL, c.last_activity DESC, c.id",
        )
        .fetch_all(&mut self.conn)
        .await?;

        rows.into_iter()
            .map(|row| {
                let (key, title, last_activity, unread, contact, profile, phone) =
                    (row.0, row.1, row.2, row.3, row.4, row.5, row.6);
                let (hidden, archived, pinned, muted_until, draft) =
                    (row.7, row.8, row.9, row.10, row.11);
                let id = conversation_from_key(&key)?;
                let title = match id {
                    ConversationId::Group(_) => title,
                    ConversationId::Direct(service_id) => Contact {
                        service_id,
                        name: contact,
                        profile_name: profile,
                        phone: None,
                    }
                    .display_name()
                    .map(str::to_owned),
                };
                Ok(ConversationSummary {
                    id,
                    title,
                    phone: phone.map(PhoneNumber::new),
                    last_activity: optional_timestamp_from_db(last_activity, "last_activity")?,
                    unread: count_from_db(unread, "unread_count")?,
                    state: ConversationState {
                        hidden: hidden.unwrap_or(false),
                        archived: archived.unwrap_or(false),
                        pinned: pinned.map(|p| count_from_db(p, "pinned")).transpose()?,
                        muted_until: optional_timestamp_from_db(muted_until, "muted_until")?,
                        draft: draft.map(MessageBody::new),
                    },
                })
            })
            .collect()
    }

    /// Up to `limit` messages older than `before` (newest page if `None`), oldest first.
    pub(super) async fn messages(
        &mut self,
        conversation: ConversationId,
        before: Option<MessageRef>,
        limit: u32,
    ) -> Result<Vec<Message>> {
        type Row = (
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
        );
        let key = conversation_key(conversation);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, sender, sent_at, body, quote_sender, quote_sent_at, quote_preview,
                    edited_at, deleted, direction, status
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
            let (id, sender, sent_at, body, quote_sender, quote_sent_at, quote_preview) =
                (row.0, row.1, row.2, row.3, row.4, row.5, row.6);
            let (edited_at, deleted, direction, status) = (row.7, row.8, row.9, row.10);
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
            ids.push(id);
            messages.push(Message {
                conversation,
                sender: service_id_from_db(&sender, "sender")?,
                sent_at: timestamp_from_db(sent_at, "sent_at")?,
                direction: direction_from_db(&direction, status.as_deref())?,
                body: body.map(MessageBody::new),
                quote,
                attachments: vec![],
                reactions: vec![],
                edited_at: optional_timestamp_from_db(edited_at, "edited_at")?,
                deleted,
            });
        }
        self.attach_extras(&key, &ids, &mut messages).await?;
        Ok(messages)
    }

    /// Fill in reactions and attachments for a page of messages (`ids[i]` is the
    /// row of `messages[i]`).
    async fn attach_extras(
        &mut self,
        key: &str,
        ids: &[i64],
        messages: &mut [Message],
    ) -> Result<()> {
        let (Some(&first), Some(&last)) = (ids.first(), ids.last()) else {
            return Ok(());
        };
        let index: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let (low, high) = (first.min(last), first.max(last));
        // Row IDs of a page aren't contiguous, but filtering by conversation and the
        // page's row-ID range, then by `index`, avoids a variable-length IN list.
        let reactions: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT r.message_id, r.sender, r.emoji FROM reactions r
             JOIN messages m ON m.id = r.message_id
             WHERE m.conversation_id = ?1 AND m.id BETWEEN ?2 AND ?3
             ORDER BY r.rowid",
        )
        .bind(key)
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

        let attachments: Vec<AttachmentRow> = sqlx::query_as(
            "SELECT a.message_id, a.backend_id, a.content_type, a.file_name, a.size
             FROM attachments a JOIN messages m ON m.id = a.message_id
             WHERE m.conversation_id = ?1 AND m.id BETWEEN ?2 AND ?3
             ORDER BY a.id",
        )
        .bind(key)
        .bind(low)
        .bind(high)
        .fetch_all(&mut self.conn)
        .await?;
        for (id, backend_id, content_type, file_name, size) in attachments {
            if let Some(message) = index.get(&id).and_then(|&i| messages.get_mut(i)) {
                let size = size
                    .map(|s| u64::try_from(s).map_err(|_| StoreError::Corrupt("attachment size")))
                    .transpose()?;
                message.attachments.push(AttachmentRef {
                    id: AttachmentId(backend_id),
                    content_type,
                    file_name,
                    size,
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
                 (conversation_id, hidden, archived, pinned, muted_until, draft)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (conversation_id) DO UPDATE SET
                 hidden = excluded.hidden, archived = excluded.archived,
                 pinned = excluded.pinned, muted_until = excluded.muted_until,
                 draft = excluded.draft",
        )
        .bind(&key)
        .bind(state.hidden)
        .bind(state.archived)
        .bind(state.pinned.map(i64::from))
        .bind(state.muted_until.map(timestamp_to_db).transpose()?)
        .bind(state.draft.as_ref().map(MessageBody::expose))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn mark_read(&mut self, conversation: ConversationId) -> Result<()> {
        sqlx::query("UPDATE conversations SET unread_count = 0 WHERE id = ?1")
            .bind(conversation_key(conversation))
            .execute(&mut self.conn)
            .await?;
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

async fn insert_attachment(
    conn: &mut SqliteConnection,
    message: i64,
    attachment: &AttachmentRef,
) -> Result<()> {
    let size = attachment
        .size
        .map(|s| i64::try_from(s).map_err(|_| StoreError::OutOfRange("attachment size")))
        .transpose()?;
    sqlx::query(
        "INSERT INTO attachments (message_id, backend_id, content_type, file_name, size)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(message)
    .bind(&attachment.id.0)
    .bind(&attachment.content_type)
    .bind(attachment.file_name.as_deref())
    .bind(size)
    .execute(conn)
    .await?;
    Ok(())
}

fn now() -> Timestamp {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    Timestamp::from_millis(ms)
}
