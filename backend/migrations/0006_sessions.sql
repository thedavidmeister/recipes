-- Cook-decider sessions (#20): a group swipes recipes yes/no and the app tallies
-- the winners. Two tables.
--
-- Turso is the source of truth; the WebSocket room in the backend is only a
-- live-push accelerator (see session.rs). So a lost process — Render's 15-min
-- idle spin-down destroys every in-memory room at once — is a performance blip,
-- not data loss: a (re)joining client rehydrates the tally from here and listens
-- for subsequent live votes. This is the entire reason votes are persisted rather
-- than kept in RAM.

-- A decider session: a shareable channel a group votes in. `filter` is the
-- optional JSON scope that seeds each participant's feed (category / area / tag /
-- ingredient); NULL means the whole corpus. `channel_id` is a random token —
-- joining is still auth-session-gated (#25), so it names a session rather than
-- granting access. Named `decider_sessions`, not `sessions`: the latter is already
-- the auth login-session table (migration 0003).
CREATE TABLE IF NOT EXISTS decider_sessions (
    channel_id  TEXT PRIMARY KEY,
    created_by  TEXT NOT NULL,        -- the telegram_user_id that started it
    filter      TEXT,                 -- optional JSON scope; NULL = whole corpus
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);

-- One row per (session, recipe, voter): the durable vote. Re-voting UPDATEs the
-- row — a swipe is a person's current call, not an append-only log — so the tally
-- is a straight COUNT over this table and a reconnecting client sees exactly what
-- it last chose. `vote` is 1 (yes) / 0 (no).
CREATE TABLE IF NOT EXISTS votes (
    channel_id  TEXT NOT NULL,
    source      TEXT NOT NULL,
    id          TEXT NOT NULL,
    voter_id    TEXT NOT NULL,        -- the telegram_user_id that voted
    vote        INTEGER NOT NULL,     -- 1 = yes, 0 = no
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (channel_id, source, id, voter_id)
);

-- The tally always reads by channel; index that lookup.
CREATE INDEX IF NOT EXISTS votes_by_channel ON votes (channel_id);
