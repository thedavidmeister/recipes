-- A plan is born capped at thirty minutes (#163).
--
-- #80 gave the column no default, so a plan was born unbounded: the lobby's time
-- row sat on "Any" — the one setting that filters nothing — and did nothing until
-- somebody touched it, while the deck offered a five-hour braise to whoever is
-- hungry now. Half an hour is where most meals live, and widening it is one tap.
-- The create handler applies the same 1800 when a caller names no cap, so a row
-- inserted without one and a plan created without one read identically — the
-- pairing #114 made between `meal_type`'s column default and its handler default.
--
-- **Existing rows are deliberately not backfilled.** A DEFAULT applies to future
-- inserts only, and the copy below names `max_total_seconds` explicitly, so a NULL
-- stays NULL rather than being talked into 1800. Two reasons, and either alone
-- would settle it: a plan already under way has a *frozen* bound — `set_time_cap`
-- refuses to move it precisely because the roster is swiping a corpus that bound
-- defines, so a migration must not do what the handler forbids — and an open lobby
-- is showing its host a chosen pill, which would silently become a different pill
-- under a plan they already agreed on. The new default is what a plan is born as,
-- not a rule applied retroactively to plans born under the old one.
--
-- SQLite cannot ALTER a column's default, so this is the standard table rebuild.
-- It is safe to run twice: the copy reads whichever table currently answers to
-- `pick_sessions`, so a re-run after a crashed-then-retried migration lands on the
-- same rows. There is deliberately no `DROP TABLE IF EXISTS pick_sessions_rebuild`
-- in front of it — that statement is only ever reachable in the world where the
-- transaction below did NOT protect us, and in that world the scratch table is
-- where the rows are. Failing loudly on "table already exists" leaves them for a
-- human; dropping it first would delete them.
--
-- No index, trigger, view or foreign key references `pick_sessions` (`votes`,
-- `pick_voters` and `buy_checks` all carry `channel_id` as a plain column), so the
-- rebuild has nothing to recreate afterwards.
BEGIN;

CREATE TABLE pick_sessions_rebuild (
    channel_id        TEXT PRIMARY KEY,
    created_by        TEXT NOT NULL,        -- the telegram_user_id that started it
    filter            TEXT,                 -- optional JSON scope; NULL = whole corpus
    created_at        INTEGER NOT NULL DEFAULT (unixepoch()),
    kitchen_id        TEXT,
    started_at        INTEGER,              -- NULL while the lobby is open
    meal_type         TEXT NOT NULL DEFAULT 'dinner',
    additions         TEXT NOT NULL DEFAULT '[]',

    -- The one line this migration exists for. Still nullable, because NULL still
    -- means "Any" — the default is where a plan starts, not a floor it cannot go
    -- under, and an explicit NULL (from the lobby, or from a create body that says
    -- `null`) overrides it exactly as it always did.
    max_total_seconds INTEGER DEFAULT 1800
);

INSERT INTO pick_sessions_rebuild
    (channel_id, created_by, filter, created_at, kitchen_id, started_at,
     meal_type, additions, max_total_seconds)
SELECT channel_id, created_by, filter, created_at, kitchen_id, started_at,
       meal_type, additions, max_total_seconds
FROM pick_sessions;

DROP TABLE pick_sessions;
ALTER TABLE pick_sessions_rebuild RENAME TO pick_sessions;

COMMIT;
