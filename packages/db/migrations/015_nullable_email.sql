-- Migration 015: Make users.email nullable for Clerk auth integration
--
-- Clerk's default JWT does not include the user's email claim. Auto-creating a
-- user row on first sign-in would fail if email is NOT NULL. Making it nullable
-- lets us create users from the Clerk external_id (sub claim) alone.
-- The unique constraint is also dropped: email uniqueness is enforced by Clerk;
-- multiple NULL values are allowed under UNIQUE but removing the constraint
-- makes intent explicit and avoids surprises when email is not always present.

ALTER TABLE users ALTER COLUMN email DROP NOT NULL;
ALTER TABLE users DROP CONSTRAINT IF EXISTS users_email_key;
