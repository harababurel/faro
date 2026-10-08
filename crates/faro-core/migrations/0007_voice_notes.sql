-- M10 (D80, D82): voice note flag, duration, waveform, persisted transcript,
-- and per-conversation permission for remote transcription.

ALTER TABLE attachments
    ADD COLUMN voice_note INTEGER NOT NULL DEFAULT 0 CHECK (voice_note IN (0, 1));
ALTER TABLE attachments
    ADD COLUMN duration_secs INTEGER CHECK (duration_secs >= 0);
ALTER TABLE attachments
    ADD COLUMN waveform BLOB;
ALTER TABLE attachments
    ADD COLUMN transcript TEXT;
ALTER TABLE attachments
    ADD COLUMN transcript_source TEXT
        CHECK (transcript_source IN ('local', 'remote'));

ALTER TABLE conversation_state
    ADD COLUMN remote_transcription INTEGER NOT NULL DEFAULT 0
        CHECK (remote_transcription IN (0, 1));
