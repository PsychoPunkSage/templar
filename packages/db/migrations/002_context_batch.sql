-- Migration 002: Async batch context ingestion tables
--
-- Supports multi-entry text and file upload ingestion via a Redis job queue.
-- Each batch tracks overall progress; each item tracks per-entry status.
-- Completed batches auto-expire after 7 days (TTL cleanup via worker).

-- ============================================================
-- context_ingest_batches
-- ============================================================
CREATE TABLE IF NOT EXISTS context_ingest_batches (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source_type TEXT        NOT NULL,           -- 'text' | 'file'
    filename    TEXT,                           -- NULL for text input; filename for uploads
    total       INT         NOT NULL,
    pending     INT         NOT NULL,
    succeeded   INT         NOT NULL DEFAULT 0,
    failed      INT         NOT NULL DEFAULT 0,
    status      TEXT        NOT NULL DEFAULT 'queued',  -- queued | processing | done
    expires_at  TIMESTAMPTZ,                    -- set to NOW()+7d when status='done'
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ingest_batches_user   ON context_ingest_batches(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_ingest_batches_expiry ON context_ingest_batches(expires_at)
    WHERE expires_at IS NOT NULL;  -- partial index — only rows eligible for TTL cleanup

-- ============================================================
-- context_ingest_items
-- ============================================================
CREATE TABLE IF NOT EXISTS context_ingest_items (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    batch_id    UUID        NOT NULL REFERENCES context_ingest_batches(id) ON DELETE CASCADE,
    user_id     UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    entry_index INT         NOT NULL,
    entry_text  TEXT        NOT NULL,
    status      TEXT        NOT NULL DEFAULT 'pending',  -- pending | processing | succeeded | failed
    error_msg   TEXT,                           -- populated on failure
    entry_id    UUID,                           -- populated on success (stable context entry UUID)
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ingest_items_batch ON context_ingest_items(batch_id);
