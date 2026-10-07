-- faro's own database: history and UI state (DESIGN.md §6, D7, D34).
-- presage keeps keys and sessions in its own store; nothing here is secret
-- protocol state, but message bodies and phone numbers are private (file is 0600).
--
-- Conventions:
-- - Timestamps are milliseconds since the Unix epoch (Signal's unit).
-- - Service IDs use Signal's string form (bare ACI UUID, or `PNI:<uuid>`).
-- - Conversation IDs are `direct:<service id>` or `group:<64 hex chars>`.
-- - STRICT tables, so a wrong type is an error instead of a silent conversion.

CREATE TABLE recipients (
    service_id   TEXT PRIMARY KEY,
    contact_name TEXT,
    profile_name TEXT,
    phone        TEXT
) STRICT;

CREATE TABLE conversations (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL CHECK (kind IN ('direct', 'group')),
    -- Direct only: the other person (their name comes from `recipients`).
    peer          TEXT REFERENCES recipients (service_id),
    -- Groups only.
    title         TEXT,
    -- `sent_at` of the newest message; NULL while the conversation is empty.
    last_activity INTEGER,
    unread_count  INTEGER NOT NULL DEFAULT 0 CHECK (unread_count >= 0),
    CHECK ((kind = 'direct') = (peer IS NOT NULL))
) STRICT;

CREATE TABLE group_members (
    group_id   TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    service_id TEXT NOT NULL,
    PRIMARY KEY (group_id, service_id)
) STRICT;

-- Local UI choices about a conversation. A missing row means all defaults.
CREATE TABLE conversation_state (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations (id) ON DELETE CASCADE,
    hidden          INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1)),
    archived        INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    -- Position among pinned conversations (lower first); NULL = not pinned.
    pinned          INTEGER,
    muted_until     INTEGER,
    draft           TEXT
) STRICT;

CREATE TABLE messages (
    id              INTEGER PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    sender          TEXT NOT NULL,
    sent_at         INTEGER NOT NULL,
    -- When this database first saw the message (local clock).
    received_at     INTEGER NOT NULL,
    -- NULL for attachment-only messages and after deletion.
    body            TEXT,
    quote_sender    TEXT,
    quote_sent_at   INTEGER,
    quote_preview   TEXT,
    edited_at       INTEGER,
    deleted         INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    direction       TEXT NOT NULL CHECK (direction IN ('incoming', 'outgoing')),
    -- Delivery progress of outgoing messages; always NULL for incoming ones.
    status          TEXT CHECK (status IN ('sending', 'sent', 'delivered', 'read', 'failed')),
    -- Signal identifies a message by its author and the author's timestamp.
    UNIQUE (sender, sent_at),
    CHECK ((direction = 'incoming') = (status IS NULL)),
    CHECK ((quote_sender IS NULL) = (quote_sent_at IS NULL))
) STRICT;

-- History pages: newest first within a conversation, `(sent_at, sender)` as the cursor.
CREATE INDEX messages_by_conversation ON messages (conversation_id, sent_at, sender);

CREATE TABLE reactions (
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    sender     TEXT NOT NULL,
    emoji      TEXT NOT NULL,
    PRIMARY KEY (message_id, sender)
) STRICT;

CREATE TABLE attachments (
    id           INTEGER PRIMARY KEY,
    message_id   INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    -- Backend-specific handle used to fetch the content (`AttachmentId`).
    backend_id   TEXT NOT NULL,
    content_type TEXT NOT NULL,
    file_name    TEXT,
    size         INTEGER CHECK (size >= 0),
    -- Where the downloaded content lives; NULL until fetched.
    local_path   TEXT
) STRICT;

CREATE INDEX attachments_by_message ON attachments (message_id);

-- Small UI facts that outlive a session: last open conversation, sidebar widths…
CREATE TABLE app_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
