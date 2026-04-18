-- Migration 016: Multi-page / CV mode support
--
-- Adds:
--   resumes.resume_type   TEXT NOT NULL DEFAULT 'single_page'
--   resumes.page_count    SMALLINT      (null until generation sets it)
--   resume_bullets.page_number SMALLINT NOT NULL DEFAULT 1
--   cv_projects.document_type  TEXT NOT NULL DEFAULT 'single_page'

ALTER TABLE resumes
    ADD COLUMN IF NOT EXISTS resume_type TEXT NOT NULL DEFAULT 'single_page',
    ADD COLUMN IF NOT EXISTS page_count  SMALLINT;

ALTER TABLE resume_bullets
    ADD COLUMN IF NOT EXISTS page_number SMALLINT NOT NULL DEFAULT 1;

ALTER TABLE cv_projects
    ADD COLUMN IF NOT EXISTS document_type TEXT NOT NULL DEFAULT 'single_page';

-- Index for multi-page bullet ordering
CREATE INDEX IF NOT EXISTS idx_resume_bullets_page_section
    ON resume_bullets (resume_id, page_number, section, order_idx);
