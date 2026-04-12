-- Migration 014: add entry_groups column to resumes table
-- Stores serialized Vec<EntryGroup> for page-reload restoration of typed display headers.
-- NULL for resumes created before this migration (pre-FIX-10).
ALTER TABLE resumes ADD COLUMN IF NOT EXISTS entry_groups JSONB;

COMMENT ON COLUMN resumes.entry_groups IS
    'Serialized Vec<EntryGroup> — typed display headers (company/role/dates) for
     page-reload restoration in the frontend editor. NULL for legacy resumes.';
