-- Buy (#131): the shopping checklist belongs to the **meal**, not the browser.
--
-- Ticks used to live in `localStorage`, which made them private to one device and
-- attributable to nobody: two people shopping the same meal each ticked their own
-- copy and neither ever saw the other's. Moving them here is what makes "who
-- grabbed the carrots" a question the app can answer at all — the meal session is
-- already where the pick consensus and the lobby's deciders live, so the list that
-- follows from that decision belongs beside them.
--
-- One row per ticked item; an untick is a DELETE. A row's *existence* is the tick,
-- so there is no "unticked" state to store and no tombstones to reconcile.
CREATE TABLE IF NOT EXISTS buy_checks (
    channel_id        TEXT NOT NULL,    -- the meal session (pick_sessions.channel_id)
    source            TEXT NOT NULL,    -- the consensus recipe
    id                TEXT NOT NULL,
    ingredient_index  INTEGER NOT NULL, -- which line of that recipe's ingredient list
    user_id           TEXT NOT NULL,    -- the telegram_user_id that ticked it
    created_at        INTEGER NOT NULL DEFAULT (unixepoch()),

    -- `user_id` is deliberately NOT in the key. An item in a basket is in exactly
    -- one basket: the thing being recorded is "this is got", and the person is the
    -- attribution on it, not a second dimension of it. So a tick is idempotent (the
    -- same person tapping twice is one row), a second person tapping the same line
    -- takes it over rather than adding a duplicate — last writer wins, the way a
    -- shared list works out loud — and the checklist can never show one ingredient
    -- claimed by two people at once.
    PRIMARY KEY (channel_id, source, id, ingredient_index)
);

-- No secondary index: the checklist is always read by
-- (channel_id, source, id), which is a prefix of the primary key, so SQLite's
-- automatic index on it already serves that lookup. A second index over the same
-- columns would only cost writes.
