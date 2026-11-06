-- Identity keys and safety-number verification (D163).
ALTER TABLE recipients ADD COLUMN safety_number TEXT;
ALTER TABLE recipients ADD COLUMN verified TEXT NOT NULL DEFAULT 'default'
    CHECK (verified IN ('default', 'verified', 'unverified'));
ALTER TABLE recipients ADD COLUMN key_changed INTEGER NOT NULL DEFAULT 0
    CHECK (key_changed IN (0, 1));
