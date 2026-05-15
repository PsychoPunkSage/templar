-- Migration 021: Behavioral analytics profile table
--
-- Nightly-computed per-user signals derived from the ClickHouse event stream.
-- Read by the generation service at the start of every generate_resume() call (< 5ms via PK lookup).
-- Written only by the analytics service nightly aggregation job — never by the API directly.

CREATE TABLE user_behavioral_profile (
    user_id                 UUID        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    preferred_template      TEXT,
    avg_regen_count         FLOAT       NOT NULL DEFAULT 0.0,
    tone_correction_signal  FLOAT       NOT NULL DEFAULT 0.0,
    suppressed_sections     TEXT[]      NOT NULL DEFAULT '{}',
    common_edit_sections    TEXT[]      NOT NULL DEFAULT '{}',
    callback_proxy_rate     FLOAT       NOT NULL DEFAULT 0.0,
    last_computed_at        TIMESTAMPTZ,
    decay_factor            FLOAT       NOT NULL DEFAULT 1.0,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);
