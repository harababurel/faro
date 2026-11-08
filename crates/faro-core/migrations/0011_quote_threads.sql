-- Quote thread lens (D166): index quote parents per conversation so walking
-- down a quote tree (direct and transitive replies) never scans the whole
-- conversation history.
CREATE INDEX messages_by_quote
    ON messages (conversation_id, quote_sender, quote_sent_at)
    WHERE quote_sender IS NOT NULL AND quote_sent_at IS NOT NULL;
