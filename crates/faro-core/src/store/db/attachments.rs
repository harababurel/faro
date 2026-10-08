//! Attachment SQL (M9, D75, D78): inserting, refreshing ids on replay, loading
//! with a message page, and saving download results.

use std::collections::HashMap;

use sqlx::sqlite::SqliteConnection;

use super::Db;
use crate::domain::{AttachmentId, AttachmentRef, Message, MessageRef};
use crate::store::codec::{
    attachment_status_from_db, attachment_status_to_db, count_from_db, timestamp_to_db,
};
use crate::store::{Result, StoreError};

/// `(message_id, backend_id, content_type, file_name, size, width, height, status, local_path)`.
type AttachmentRow = (
    i64,
    String,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    String,
    Option<String>,
);

pub(super) async fn insert(
    conn: &mut SqliteConnection,
    message: i64,
    attachment: &AttachmentRef,
) -> Result<()> {
    let size = size_to_db(attachment.size)?;
    let (status, local_path) = attachment_status_to_db(&attachment.status)?;
    sqlx::query(
        "INSERT INTO attachments
             (message_id, backend_id, content_type, file_name, size, width, height, status, local_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(message)
    .bind(&attachment.id.0)
    .bind(&attachment.content_type)
    .bind(attachment.file_name.as_deref())
    .bind(size)
    .bind(attachment.width.map(i64::from))
    .bind(attachment.height.map(i64::from))
    .bind(status)
    .bind(local_path)
    .execute(conn)
    .await?;
    Ok(())
}

/// When an already-stored message is replayed on startup (D64, D75), heal any
/// pre-M9 attachment rows by updating their `backend_id` and missing dimensions
/// by position, keeping their download status untouched.
pub(super) async fn refresh(
    conn: &mut SqliteConnection,
    target: MessageRef,
    attachments: &[AttachmentRef],
) -> Result<()> {
    let Some(message) = super::message_id(&mut *conn, target).await? else {
        return Ok(());
    };
    let rows: Vec<(i64,)> =
        sqlx::query_as("SELECT id FROM attachments WHERE message_id = ?1 ORDER BY id")
            .bind(message)
            .fetch_all(&mut *conn)
            .await?;
    for ((id,), attachment) in rows.into_iter().zip(attachments) {
        if attachment.id.0.is_empty() {
            continue;
        }
        sqlx::query(
            "UPDATE attachments
             SET backend_id = ?1,
                 width = coalesce(width, ?2),
                 height = coalesce(height, ?3)
             WHERE id = ?4",
        )
        .bind(&attachment.id.0)
        .bind(attachment.width.map(i64::from))
        .bind(attachment.height.map(i64::from))
        .bind(id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

pub(super) async fn load(
    conn: &mut SqliteConnection,
    key: &str,
    low: i64,
    high: i64,
    index: &HashMap<i64, usize>,
    messages: &mut [Message],
) -> Result<()> {
    let rows: Vec<AttachmentRow> = sqlx::query_as(
        "SELECT a.message_id, a.backend_id, a.content_type, a.file_name, a.size,
                a.width, a.height, a.status, a.local_path
         FROM attachments a JOIN messages m ON m.id = a.message_id
         WHERE m.conversation_id = ?1 AND m.id BETWEEN ?2 AND ?3
         ORDER BY a.id",
    )
    .bind(key)
    .bind(low)
    .bind(high)
    .fetch_all(conn)
    .await?;
    for row in rows {
        let (id, backend_id, content_type, file_name, size, width, height, status, local_path) =
            row;
        let Some(message) = index.get(&id).and_then(|&i| messages.get_mut(i)) else {
            continue;
        };
        let size = size
            .map(|s| u64::try_from(s).map_err(|_| StoreError::Corrupt("attachment size")))
            .transpose()?;
        let width = width
            .map(|w| count_from_db(w, "attachment width"))
            .transpose()?;
        let height = height
            .map(|h| count_from_db(h, "attachment height"))
            .transpose()?;
        let status = attachment_status_from_db(&status, local_path)?;
        message.attachments.push(AttachmentRef {
            id: AttachmentId(backend_id),
            content_type,
            file_name,
            size,
            width,
            height,
            status,
        });
    }
    Ok(())
}

impl Db {
    /// Update the `index`-th attachment on `message` (download status, path,
    /// and any size or dimensions learned from the downloaded file, D75).
    pub(in crate::store) async fn set_attachment(
        &mut self,
        message: MessageRef,
        index: usize,
        attachment: &AttachmentRef,
    ) -> Result<()> {
        let offset = i64::try_from(index).map_err(|_| StoreError::OutOfRange("attachment"))?;
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT a.id FROM attachments a
             JOIN messages m ON m.id = a.message_id
             WHERE m.sender = ?1 AND m.sent_at = ?2
             ORDER BY a.id
             LIMIT 1 OFFSET ?3",
        )
        .bind(message.sender.to_string())
        .bind(timestamp_to_db(message.sent_at)?)
        .bind(offset)
        .fetch_optional(&mut self.conn)
        .await?;
        let Some((id,)) = row else {
            tracing::debug!("attachment update for an unknown message or index");
            return Ok(());
        };
        let size = size_to_db(attachment.size)?;
        let (status, local_path) = attachment_status_to_db(&attachment.status)?;
        sqlx::query(
            "UPDATE attachments
             SET size = coalesce(?1, size),
                 width = coalesce(?2, width),
                 height = coalesce(?3, height),
                 status = ?4,
                 local_path = ?5
             WHERE id = ?6",
        )
        .bind(size)
        .bind(attachment.width.map(i64::from))
        .bind(attachment.height.map(i64::from))
        .bind(status)
        .bind(local_path)
        .bind(id)
        .execute(&mut self.conn)
        .await?;
        Ok(())
    }
}

fn size_to_db(size: Option<u64>) -> Result<Option<i64>> {
    size.map(|s| i64::try_from(s).map_err(|_| StoreError::OutOfRange("attachment size")))
        .transpose()
}
