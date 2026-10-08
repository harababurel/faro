-- Attachment dimensions and download status (M9, D75, D78). `local_path`
-- already exists; "downloading" is in memory only.
ALTER TABLE attachments
    ADD COLUMN width INTEGER CHECK (width >= 0);
ALTER TABLE attachments
    ADD COLUMN height INTEGER CHECK (height >= 0);
ALTER TABLE attachments
    ADD COLUMN status TEXT NOT NULL DEFAULT 'remote'
    CHECK (status IN ('remote', 'downloaded', 'failed'));
