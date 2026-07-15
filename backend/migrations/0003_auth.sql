-- Auth (#25). Mandatory: every API endpoint requires a session, so these tables
-- sit in front of the whole corpus.
--
-- Identity is a Telegram user id. Auth exists because #20 needs a headcount —
-- whose vote is whose — not to protect the corpus, which the adapter gate
-- already does by construction.

-- A person. Bound to the Telegram *user id*, never the username: usernames are
-- mutable and reassignable, so a username-keyed account could be silently
-- inherited by whoever claims a released handle. `username` is stored only to
-- show a human-readable name, and is refreshed on each login.
CREATE TABLE IF NOT EXISTS users (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    telegram_user_id TEXT    NOT NULL UNIQUE,
    username         TEXT,
    created_at       INTEGER NOT NULL DEFAULT (unixepoch())
);

-- One login attempt: the browser mints it, Telegram claims it.
--
-- Two secrets, deliberately: `nonce` travels in a t.me link and is therefore
-- *shareable* (the user may screenshot it, and #20 wants the bot posting links
-- into group chats), while `poll_secret` never leaves the browser that minted
-- it. If a single value did both jobs, anyone who saw the link could redeem the
-- session it mints. Split, a shared link only lets someone claim the attempt
-- with their OWN Telegram id — it never hands them someone else's session.
--
-- Only hashes are stored: a leaked DB must not yield a usable login. They are
-- 256-bit random, so the hash is also the lookup key — we never compare a secret
-- in application code, which is what a constant-time compare would have been
-- protecting.
CREATE TABLE IF NOT EXISTS login_attempts (
    nonce_hash       TEXT    PRIMARY KEY,
    poll_secret_hash TEXT    NOT NULL UNIQUE,
    created_at       INTEGER NOT NULL DEFAULT (unixepoch()),
    -- Short TTL: an unclaimed attempt is a live credential until it expires.
    expires_at       INTEGER NOT NULL,
    -- Set when the bot receives `/start <nonce>`. NULL means unclaimed.
    telegram_user_id TEXT,
    username         TEXT,
    claimed_at       INTEGER
);

-- Claiming and polling both look up by hash; expiry sweeps scan by time.
CREATE INDEX IF NOT EXISTS login_attempts_expires_idx ON login_attempts (expires_at);

-- An issued session. The token is 256-bit random and stored only as a hash, so
-- the same reasoning as above applies: a DB leak yields no usable session.
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT    PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    expires_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS sessions_user_idx ON sessions (user_id);
CREATE INDEX IF NOT EXISTS sessions_expires_idx ON sessions (expires_at);
