-- Seed: dev user for pre-auth MVP
--
-- This user is hardcoded in API handlers (00000000-0000-0000-0000-000000000001)
-- until Clerk auth is wired in Phase 9. This file must run AFTER 001_initial.sql
-- so that the users table already exists.
--
-- Production: real users are created via Clerk webhook; this seed row is dev-only.

INSERT INTO users (id, external_id, email, tier)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'dev_user_001',
    'dev@templar.local',
    'pro'
)
ON CONFLICT (id) DO NOTHING;
