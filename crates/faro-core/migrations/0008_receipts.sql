-- M12 (D85): per-recipient delivery and read receipts for outgoing messages.
CREATE TABLE message_receipts (
    sender      TEXT    NOT NULL,
    sent_at     INTEGER NOT NULL,
    recipient   TEXT    NOT NULL,
    status      TEXT    NOT NULL CHECK (status IN ('delivered', 'read')),
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (sender, sent_at, recipient),
    FOREIGN KEY (sender, sent_at) REFERENCES messages (sender, sent_at) ON DELETE CASCADE
) STRICT;
