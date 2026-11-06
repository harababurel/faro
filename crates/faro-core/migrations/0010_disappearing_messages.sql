-- Disappearing messages (D165).
ALTER TABLE conversations ADD COLUMN disappearing_timer INTEGER
    CHECK (disappearing_timer IS NULL OR disappearing_timer > 0);
ALTER TABLE messages ADD COLUMN expires_in_secs INTEGER
    CHECK (expires_in_secs IS NULL OR expires_in_secs > 0);
ALTER TABLE messages ADD COLUMN expire_started_at INTEGER;

CREATE INDEX messages_expiring ON messages (expire_started_at, expires_in_secs)
    WHERE expires_in_secs IS NOT NULL;
