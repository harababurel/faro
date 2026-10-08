-- Mentions (D72): who is mentioned where in a message's body. `start` and
-- `length` are byte offsets into `messages.body` (Signal sends UTF-16
-- offsets; the backend converts them).
CREATE TABLE mentions (
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    start      INTEGER NOT NULL,
    length     INTEGER NOT NULL,
    service_id TEXT NOT NULL,
    PRIMARY KEY (message_id, start)
) STRICT;
