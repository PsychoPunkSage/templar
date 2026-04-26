-- Migration 019: Cover letters table + current_cover_letter_id on cv_projects
--
-- cover_letters: stores generated cover letter content as structured JSONB paragraphs.
-- Each cover letter is linked to a user and optionally to a resume and persona.
-- One cover letter per project (current_cover_letter_id FK on cv_projects).
-- jd_text_hash enables frontend staleness detection without re-fetching full JD text.

CREATE TABLE cover_letters (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID        NOT NULL,
    resume_id    UUID        REFERENCES resumes(id) ON DELETE SET NULL,
    persona_id   UUID        REFERENCES personas(id) ON DELETE SET NULL,
    jd_text_hash TEXT        NOT NULL,
    tone         TEXT        NOT NULL DEFAULT 'formal',
    focus        TEXT        NOT NULL DEFAULT 'technical',
    -- [{role: "hook"|"fit"|"culture"|"close", text: "..."}]
    content      JSONB       NOT NULL,
    company_name TEXT,
    role_title   TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX cover_letters_user_id_idx ON cover_letters(user_id);
CREATE INDEX cover_letters_resume_id_idx ON cover_letters(resume_id)
    WHERE resume_id IS NOT NULL;

ALTER TABLE cv_projects
    ADD COLUMN IF NOT EXISTS current_cover_letter_id UUID
        REFERENCES cover_letters(id) ON DELETE SET NULL;
