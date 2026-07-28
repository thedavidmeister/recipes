-- Pre-tick from the pantry (#156): a shopping list starts with the things the
-- plan's kitchen already has ticked off.
--
-- Two changes, and both are about the same thing — a tick that has no person behind
-- it. #131 built `buy_checks` on the rule that a tick is somebody's: the row records
-- "this is got" and `user_id` is the attribution on it. A pantry pre-tick is got and
-- is nobody's, so the table has to be able to say that rather than have a person
-- invented for it. Wearing a colour is what a claim looks like in this app, and the
-- cupboard did not claim anything.

-- 1. A tick's origin, as a discriminant the database enforces.
--
-- SQLite cannot drop a NOT NULL in place, so the table is rebuilt. Existing rows are
-- carried over as what they are: somebody's.
CREATE TABLE IF NOT EXISTS buy_checks_new (
    channel_id        TEXT NOT NULL,    -- the meal session (pick_sessions.channel_id)
    source            TEXT NOT NULL,    -- the consensus recipe
    id                TEXT NOT NULL,
    ingredient_index  INTEGER NOT NULL, -- which line of that recipe's shopping list

    -- Exactly one of these two is set, and the CHECK below is what makes that a fact
    -- rather than a convention. A NULL `user_id` on its own would only say "nobody",
    -- which is not the same claim: `pantry_item` says *why* nobody — the kitchen
    -- already had it, and names the entry that answered for the line.
    user_id           TEXT,             -- the telegram_user_id that ticked it
    pantry_item       TEXT,             -- the kitchen_pantry entry that pre-ticked it

    created_at        INTEGER NOT NULL DEFAULT (unixepoch()),

    -- Still keyed without the person (#131): an item in a basket is in exactly one
    -- basket. A pantry pre-tick occupies that one slot the same as anyone's tick, so
    -- ticking a pre-ticked line takes it over and unticking it is an ordinary DELETE
    -- — the jar was empty, and that must behave like any other untick.
    PRIMARY KEY (channel_id, source, id, ingredient_index),

    -- A tick is a person's or the pantry's. Never both (a claim on top of a claim),
    -- never neither (a tick nobody and nothing put there is not a state this table
    -- has — the unattributed list a solo decision falls back to is device-local and
    -- reaches no row at all, see frontend/src/lib/buy.ts).
    CHECK ((user_id IS NULL) <> (pantry_item IS NULL))
);

INSERT INTO buy_checks_new (channel_id, source, id, ingredient_index, user_id, pantry_item, created_at)
    SELECT channel_id, source, id, ingredient_index, user_id, NULL, created_at FROM buy_checks;

DROP TABLE buy_checks;

ALTER TABLE buy_checks_new RENAME TO buy_checks;

-- 2. The seed is one-time, and this is the record of it having happened.
--
-- Without a marker, "seed when the list has no ticks yet" would re-tick everything the
-- moment a shopper unticked the last line and reloaded — the pantry would keep putting
-- the empty jar back. So the fact recorded is *that this list was seeded*, not what it
-- was seeded with, and it survives the list being emptied.
--
-- It also settles recompute-vs-snapshot in the table rather than in a code path: stock
-- added to the kitchen mid-shop does not re-tick, because the seed already ran. A
-- shopping list that rearranges itself under somebody standing in an aisle is worse
-- than one that is a little out of date, and the person can tick the line themselves.
--
-- One row per (meal, recipe), not per meal: the checklist is keyed by recipe, so a
-- re-decided plan gets a fresh list and a fresh seed.
CREATE TABLE IF NOT EXISTS buy_seeds (
    channel_id TEXT NOT NULL,
    source     TEXT NOT NULL,
    id         TEXT NOT NULL,
    seeded_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (channel_id, source, id)
);
