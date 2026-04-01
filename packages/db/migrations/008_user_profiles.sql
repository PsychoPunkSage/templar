-- User profile: contact info and flexible links for PDF header rendering.
-- links is a JSONB array: [{ "type": "LinkedIn"|"GitHub"|"GitLab"|"Twitter"|"Portfolio"|"Custom",
--                             "label": "..." (Custom only),
--                             "url": "...", "alias": "..." }]
CREATE TABLE user_profiles (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    full_name  TEXT NOT NULL DEFAULT '',
    email      TEXT NOT NULL DEFAULT '',
    phone      TEXT NOT NULL DEFAULT '',
    location   TEXT NOT NULL DEFAULT '',
    links      JSONB NOT NULL DEFAULT '[]',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
