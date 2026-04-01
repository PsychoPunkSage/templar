-- Add rejection_reason column to resume_bullets for audit manifest fidelity
ALTER TABLE resume_bullets ADD COLUMN IF NOT EXISTS rejection_reason TEXT;
