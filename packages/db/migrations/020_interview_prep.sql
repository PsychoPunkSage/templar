-- Migration 020: Interview Prep tables
--
-- Two tables:
--   interview_prep_bullets  — per-bullet STAR scaffolds and questions
--   interview_prep_meta     — per-project metadata: status, gap questions, company context

CREATE TABLE interview_prep_bullets (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id        UUID        NOT NULL REFERENCES cv_projects(id) ON DELETE CASCADE,
    bullet_hash       TEXT        NOT NULL,
    bullet_text       TEXT        NOT NULL,
    context_entry_id  UUID,
    star_scaffold     JSONB       NOT NULL DEFAULT '{}',
    questions         JSONB       NOT NULL DEFAULT '[]',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, bullet_hash)
);

CREATE TABLE interview_prep_meta (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id          UUID        NOT NULL REFERENCES cv_projects(id) ON DELETE CASCADE UNIQUE,
    gap_questions       JSONB       NOT NULL DEFAULT '[]',
    company_context     JSONB,
    status              TEXT        NOT NULL DEFAULT 'pending',
    last_generated_at   TIMESTAMPTZ,
    expires_at          TIMESTAMPTZ,
    is_stale            BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX ON interview_prep_bullets (project_id);
CREATE INDEX ON interview_prep_meta (project_id);
