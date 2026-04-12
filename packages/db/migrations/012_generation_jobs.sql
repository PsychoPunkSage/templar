-- Migration 012: async generation job queue
--
-- Decouples generation from the HTTP request lifecycle.
-- The handler inserts a row and enqueues the job_id to Redis.
-- The generation worker dequeues it, calls generate_resume(), and
-- writes the full GenerateResponse as result JSONB on completion.
-- The frontend polls GET /api/v1/generation/jobs/:id/status until done.

CREATE TABLE IF NOT EXISTS generation_jobs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL,
    -- Full GenerateRequest payload stored as JSON so the worker is self-contained
    -- and can reconstruct the request without the HTTP connection being in scope.
    request     JSONB NOT NULL,
    status      TEXT NOT NULL DEFAULT 'queued',  -- queued | processing | done | failed
    error       TEXT,                             -- populated on failure
    resume_id   UUID,                             -- populated on success (FK to resumes)
    -- Full GenerateResponse payload stored on success.
    -- The status endpoint deserializes this to return fit_report + entry_groups.
    result      JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS generation_jobs_user_status ON generation_jobs (user_id, status);
