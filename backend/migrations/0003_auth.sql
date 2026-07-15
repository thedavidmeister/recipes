-- Auth (#25). Mandatory: every API endpoint requires a session, so these tables
-- sit in front of the whole corpus.
--
-- Identity is a Telegram user id. Auth exists because #20 needs a headcount —
-- whose vote is whose — not to protect the corpus, which the adapter gate
-- already does by construction.

-- A person. Bound to the Telegram *user id*, never the username: usernames are
-- mutable and reassignable, so a username-keyed account could be silently
-- inherited by whoever claims a released handle. `username` is stored only as a
-- display name, and tracks whatever Telegram last reported.
CREATE TABLE IF NOT EXISTS users (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    telegram_user_id TEXT    NOT NULL UNIQUE,
    username         TEXT,
    created_at       INTEGER NOT NULL DEFAULT (unixepoch())
);

-- A pending login, minted **by the bot for the person who messaged it** and
-- redeemed by that same person clicking the link the bot sends back.
--
-- The direction is the whole security property, and it is the opposite of the
-- obvious design. A "the browser starts a login and waits for someone to tap a
-- link" flow hands the redeeming capability to whoever *started* it, while the
-- identity comes from whoever *tapped*. Nothing ties those to the same person,
-- so an attacker starts a login, sends the link to a victim, and redeems a
-- session as them the moment they tap. That is not theoretical: it was
-- implemented, and demonstrated end-to-end, before this table replaced it.
--
-- So there is no browser-initiated attempt at all. The row is created only when
-- Telegram tells us a specific user pressed Start, and the secret that redeems
-- it is delivered to that user's private chat. Whoever holds the secret is
-- whoever the bot sent it to.
--
-- A side effect worth having: no anonymous HTTP caller can write here, so this
-- table cannot be grown by an unauthenticated request.
CREATE TABLE IF NOT EXISTS login_completions (
    -- Only the hash: a leaked db must not yield a usable login. 256-bit random,
    -- so the digest is also the lookup key and no secret is ever compared in
    -- application code.
    completion_hash  TEXT    PRIMARY KEY,
    -- Who it will log in. Comes from Telegram, never from a caller.
    telegram_user_id TEXT    NOT NULL,
    username         TEXT,
    created_at       INTEGER NOT NULL DEFAULT (unixepoch()),
    -- Short TTL: until redeemed or expired this is a live credential sitting in
    -- a chat message.
    expires_at       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS login_completions_expires_idx ON login_completions (expires_at);

-- An issued session. The token is 256-bit random and stored only as a hash, so a
-- db leak yields no usable session.
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT    PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    expires_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS sessions_user_idx ON sessions (user_id);
CREATE INDEX IF NOT EXISTS sessions_expires_idx ON sessions (expires_at);
