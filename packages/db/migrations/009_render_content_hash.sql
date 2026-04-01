ALTER TABLE resumes ADD COLUMN IF NOT EXISTS content_hash TEXT;
CREATE INDEX IF NOT EXISTS idx_resumes_content_hash ON resumes(content_hash) WHERE content_hash IS NOT NULL;
