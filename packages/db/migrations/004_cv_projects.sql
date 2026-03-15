-- Migration 004: cv_projects table + template_id on resumes
--
-- cv_projects: a workspace linking a user's intent (template choice, project name)
-- to the latest generated resume. Projects persist independently of resumes.
--
-- Design: template_id is a plain TEXT column referencing the template directory name,
-- NOT a foreign key to a DB table. Templates live on the filesystem and in memory.
-- Adding a new template = drop a folder, no migration needed.
-- If we FK'd it, every template change would require a DB migration AND a deploy.

CREATE TABLE IF NOT EXISTS cv_projects (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id           UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name              TEXT        NOT NULL,
    -- template_id is TEXT (not FK) — see design note above
    template_id       TEXT        NOT NULL DEFAULT 'generic-cv',
    -- nullable: project exists before its first generation; SET NULL on resume delete
    -- so deleting a resume doesn't cascade-delete the project workspace
    current_resume_id UUID        REFERENCES resumes(id) ON DELETE SET NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- User's project list ordered by recency — this is the primary access pattern
CREATE INDEX IF NOT EXISTS idx_cv_projects_user_id ON cv_projects(user_id, updated_at DESC);

-- Partial index: only on rows that actually have a resume (avoids indexing NULLs)
CREATE INDEX IF NOT EXISTS idx_cv_projects_resume  ON cv_projects(current_resume_id)
    WHERE current_resume_id IS NOT NULL;

-- Allow resumes to carry which template they were generated with.
-- Needed so the render worker can choose the right LaTeX code path.
-- Optional: NULL means use the legacy font-based template path.
ALTER TABLE resumes ADD COLUMN IF NOT EXISTS template_id TEXT;
