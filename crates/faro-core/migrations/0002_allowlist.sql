-- Allowlist mode (M4 task 5, D41): conversations the user chose to keep
-- showing while the mode is on.
ALTER TABLE conversation_state
    ADD COLUMN allowed INTEGER NOT NULL DEFAULT 0 CHECK (allowed IN (0, 1));
