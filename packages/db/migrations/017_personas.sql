CREATE TABLE IF NOT EXISTS personas (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id          UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name             TEXT        NOT NULL,
    emphasized_tags  TEXT[]      NOT NULL DEFAULT '{}',
    suppressed_tags  TEXT[]      NOT NULL DEFAULT '{}',
    tone_preference  TEXT,
    section_order    JSONB,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_personas_user_id
    ON personas (user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_personas_emphasized_tags
    ON personas USING GIN (emphasized_tags);

CREATE INDEX IF NOT EXISTS idx_personas_suppressed_tags
    ON personas USING GIN (suppressed_tags);
