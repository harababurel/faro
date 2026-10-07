-- Per-conversation notification level (M6, D49), alongside the mute:
-- 'all' messages, or 'mentions' only.
ALTER TABLE conversation_state
    ADD COLUMN notify TEXT NOT NULL DEFAULT 'all' CHECK (notify IN ('all', 'mentions'));
