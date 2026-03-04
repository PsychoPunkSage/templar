-- Migration 003: Phase 5.5 — Quality scoring and dedup tracking
--
-- context_entries gains quality metadata so ingest is non-blocking.
-- context_ingest_items gains merged_with to track dedup merges.

-- Quality fields on context_entries
ALTER TABLE context_entries
    ADD COLUMN IF NOT EXISTS quality_score FLOAT8 NOT NULL DEFAULT 1.0,
    ADD COLUMN IF NOT EXISTS quality_flags TEXT[]  NOT NULL DEFAULT '{}';

-- Merge tracking on context_ingest_items
-- NULL → normal insert; non-NULL → this item was merged into the referenced entry_id
ALTER TABLE context_ingest_items
    ADD COLUMN IF NOT EXISTS merged_with UUID;
