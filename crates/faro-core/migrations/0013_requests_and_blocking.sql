-- Quarantine unaccepted message requests and blocked conversations out of
-- the main conversation list (D176).
ALTER TABLE conversation_state ADD COLUMN message_request INTEGER NOT NULL DEFAULT 0;
ALTER TABLE conversation_state ADD COLUMN blocked         INTEGER NOT NULL DEFAULT 0;
