CREATE TABLE IF NOT EXISTS fit_scores (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    jd_hash      TEXT        NOT NULL,
    context_hash TEXT        NOT NULL,
    fit_report   JSONB       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_fit_score_cache UNIQUE (user_id, jd_hash, context_hash)
);
CREATE INDEX IF NOT EXISTS idx_fit_scores_lookup
    ON fit_scores(user_id, jd_hash, context_hash);

ALTER TABLE cv_projects ADD COLUMN IF NOT EXISTS last_jd_text TEXT;
