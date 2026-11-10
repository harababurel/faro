-- D177: inline text style spans (Signal BodyRange styles), UTF-8 byte offsets.
CREATE TABLE message_styles (
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    start      INTEGER NOT NULL,
    length     INTEGER NOT NULL,
    style      TEXT    NOT NULL CHECK (style IN ('bold', 'italic', 'spoiler', 'strikethrough', 'monospace')),
    PRIMARY KEY (message_id, start, length, style)
) STRICT;
