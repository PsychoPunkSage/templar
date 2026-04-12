-- Migration 011: add order_idx to resume_bullets for deterministic render ordering.
-- Preserves the relevance-ranked insertion order from the generation pipeline
-- instead of sorting by UUID (source_entry_id), which is arbitrary.
ALTER TABLE resume_bullets
    ADD COLUMN IF NOT EXISTS order_idx INTEGER NOT NULL DEFAULT 0;

COMMENT ON COLUMN resume_bullets.order_idx IS
    'Insertion rank from the generation pipeline (0-based). Used in ORDER BY for render.';
