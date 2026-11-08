-- Inline system status events in conversation timelines (group joins/leaves/renames,
-- disappearing-timer changes, and calls; D167).
ALTER TABLE messages ADD COLUMN system_event TEXT;
