-- Invites expire (#72).
--
-- A kitchen used to carry one invite token for its whole life. Every link ever shared
-- stayed live forever, and since everyone in a kitchen is an owner of it (0012), a
-- link that leaked — a screenshot, a forwarded message, a photo of a QR code on a
-- fridge — was permanent, unrevokable, full access to the room.
--
-- So an invite is now its own short-lived thing: minted when someone asks to invite,
-- good for two hours, and stored only as a hash. The same reasoning as every other
-- secret here (#25): the hash is the lookup key, so the database cannot hand anybody a
-- working link, and a stolen backup is not a set of open doors.
DROP INDEX IF EXISTS kitchens_invite_token;
ALTER TABLE kitchens DROP COLUMN invite_token;

CREATE TABLE IF NOT EXISTS kitchen_invites (
    token_hash TEXT NOT NULL PRIMARY KEY,
    kitchen_id TEXT NOT NULL,
    created_by TEXT NOT NULL,   -- telegram_user_id of whoever minted it
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
-- Both times are recorded, because one of them alone trusts the clock too much. An
-- expiry is minted as `now + 2h`, so a clock running fast mints one that outlives the
-- two hours it promised — the link is still live long after the occasion, which is the
-- thing this table exists to prevent. Redemption therefore also refuses anything
-- claiming to have been created in the future: once the clock is corrected, an invite
-- minted during the excursion is dead rather than long-lived. A slow clock mints one
-- that dies early, which is the safe direction and needs no guard.
--
-- Expiry is the check on redemption and how the sweep finds the dead ones — both the
-- lapsed and the impossible, since an expiry further out than an invite could honestly
-- reach would otherwise never lapse and would sit here forever.
CREATE INDEX IF NOT EXISTS kitchen_invites_expiry ON kitchen_invites (expires_at);
