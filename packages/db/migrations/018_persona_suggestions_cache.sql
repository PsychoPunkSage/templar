CREATE TABLE IF NOT EXISTS persona_suggestions_cache (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    context_hash TEXT        NOT NULL,
    persona_hash TEXT        NOT NULL,
    suggestions  JSONB       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_persona_suggestion_cache UNIQUE (user_id, context_hash, persona_hash)
);
CREATE INDEX IF NOT EXISTS idx_persona_suggestions_lookup
    ON persona_suggestions_cache(user_id, context_hash, persona_hash);
