-- Enable UUID generation
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- users
--   ├── context_entries (versioned career data)
--   │       └── context_snapshots (frozen compiled context)
--   │
--   ├── personas (how to shape the resume)
--   │
--   └── resumes (job-specific output)
--           ├── resume_bullets (each grounded to context_entries)
--           └── render_jobs (PDF generation pipeline)

-- ============================================================
-- users
-- ============================================================
CREATE TABLE users (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    external_id TEXT NOT NULL UNIQUE,          -- Clerk user ID
    email       TEXT NOT NULL UNIQUE,
    tier        TEXT NOT NULL DEFAULT 'free',  -- free | pro | team | api
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================
-- context_entries (append-only versioned log): How context will be stored
-- ============================================================
-- (user_id, entry_id, version) must be unique
CREATE TABLE context_entries (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id           UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    entry_id          UUID        NOT NULL,   -- stable ID across versions
    version           INT         NOT NULL,
    entry_type        TEXT        NOT NULL,   -- experience | education | project | skill | ...
    data              JSONB       NOT NULL,   -- structured entry data
    raw_text          TEXT,                   -- original user input
    recency_score     FLOAT8      NOT NULL DEFAULT 1.0,
    impact_score      FLOAT8      NOT NULL DEFAULT 0.5,
    tags              TEXT[]      NOT NULL DEFAULT '{}',
    flagged_evergreen BOOLEAN     NOT NULL DEFAULT FALSE,
    contribution_type TEXT        NOT NULL DEFAULT 'team_member',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (user_id, entry_id, version)
);

CREATE INDEX idx_context_entries_user_id ON context_entries(user_id);
CREATE INDEX idx_context_entries_entry_type ON context_entries(entry_type);
CREATE INDEX idx_context_entries_tags ON context_entries USING GIN(tags);
CREATE INDEX idx_context_entries_data ON context_entries USING GIN(data);

-- ============================================================
-- context_snapshots: context file,to be used by AI to generate resumes
-- ============================================================
CREATE TABLE context_snapshots (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    version    INT         NOT NULL,
    s3_key     TEXT        NOT NULL,   -- contexts/{user_id}/v{version}.md
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (user_id, version)
);

CREATE INDEX idx_context_snapshots_user_id ON context_snapshots(user_id);

-- ============================================================
-- resumes
-- ============================================================
CREATE TABLE resumes (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    jd_text      TEXT        NOT NULL,
    jd_parsed    JSONB,                  -- structured JD after parsing
    fit_score    FLOAT8,                 -- 0.0-1.0 match score
    latex_source TEXT,                  -- raw LaTeX string
    s3_pdf_key   TEXT,                  -- generated PDF in S3
    status       TEXT        NOT NULL DEFAULT 'draft',  -- draft | rendered | failed
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_resumes_user_id ON resumes(user_id);
CREATE INDEX idx_resumes_status ON resumes(status);

-- ============================================================
-- resume_bullets (grounding audit)
-- ============================================================
CREATE TABLE resume_bullets (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    resume_id        UUID        NOT NULL REFERENCES resumes(id) ON DELETE CASCADE,
    section          TEXT        NOT NULL,
    bullet_text      TEXT        NOT NULL,
    source_entry_id  UUID        NOT NULL,   -- must trace to context_entries.entry_id
    grounding_score  FLOAT8      NOT NULL,   -- min 0.80 enforced by app layer
    is_user_edited   BOOLEAN     NOT NULL DEFAULT FALSE,
    line_count       SMALLINT    NOT NULL DEFAULT 1,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_resume_bullets_resume_id ON resume_bullets(resume_id);

-- ============================================================
-- render_jobs
-- ============================================================
CREATE TABLE render_jobs (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    resume_id     UUID        NOT NULL REFERENCES resumes(id) ON DELETE CASCADE,
    status        TEXT        NOT NULL DEFAULT 'queued',  -- queued | processing | done | failed
    error_message TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_render_jobs_resume_id ON render_jobs(resume_id);
CREATE INDEX idx_render_jobs_status ON render_jobs(status);

-- ============================================================
-- personas
-- ============================================================
CREATE TABLE personas (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id           UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name              TEXT        NOT NULL,
    emphasized_tags   TEXT[]      NOT NULL DEFAULT '{}',
    suppressed_tags   TEXT[]      NOT NULL DEFAULT '{}',
    tone_preference   TEXT,
    section_order     JSONB,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_personas_user_id ON personas(user_id);


