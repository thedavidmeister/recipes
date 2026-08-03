-- Cook timers are plan state (#208): one kitchen, one pot, one countdown.
--
-- A cook timer used to be a deadline in component state, persisted per recipe in the
-- browser's `localStorage`. Whoever tapped Start got a countdown and everybody else's
-- screen showed the step idle — two phones, two truths about the same 30 minutes. The
-- people cooking are the people who picked and shopped together, so the timer belongs
-- where the roster, the decision and the shopping ticks already live.
--
-- ## Numbering: why 27
--
-- `db.rs` applies by `MAX(version)`, so a number at or below one already applied on
-- production never runs at all. Production `_migrations` is at **26** (0026, #201), so
-- 27 is the next number above everything. Two other branches are in flight (#206 auth,
-- #207 kitchens); neither carries a migration as this was written, and if one lands 27
-- first this file is renumbered rather than merged into the same slot.
--
-- ## The row IS the timer
--
-- No `dismissed` column and no `done` column, for the reason 0018 gives about ticks: a
-- row's *existence* is the timer, so dismissing is a DELETE and there is no tombstone
-- to reconcile. **Done is not stored either** — a timer is done when its deadline has
-- passed, which is a reading of two numbers every client can already do and which stays
-- true with nobody connected to write it down. Storing "done" would need a writer at
-- the moment nothing is guaranteed to be running (Render spins the box down at 15
-- minutes idle), and a `done` that a crashed process failed to set would be a timer
-- that silently un-finished. See #208's "honest states, no invented data": a timer
-- nobody started simply is not here.
--
-- ## Two stored facts, and the deadline derived from them
--
-- `started_at_ms` is **the initiator's tap**, normalised into the shared timeline by
-- `events::normalize` (the app's time-sync framework — see `backend/src/events.rs`).
-- It is not the server's receipt time: the person who taps Start owns *when*, and the
-- network's latency between the tap and this row must not shift a room's countdown.
--
-- `seconds` is **the recipe's own duration**, read out of `recipes.steps` server-side
-- at the moment of the start (`timers::step_seconds`). It is never taken from the
-- wire — the initiator owns *when*, never *how long*, or one phone could set a
-- 30-minute step to 3 seconds for everybody in the room.
--
-- The **deadline is derived** (`started_at_ms + seconds * 1000`) in one place,
-- `timers::load`, rather than stored as a third column that could disagree with the two
-- it came from — the same rule #162 applies to per-serving calories.
--
-- Milliseconds, not the `unixepoch()` seconds the other session tables use: a countdown
-- is rendered to the second and re-anchored on every frame, so a second of quantisation
-- at the anchor is a second of disagreement between two phones, visible.
CREATE TABLE IF NOT EXISTS plan_timers (
    channel_id    TEXT NOT NULL,    -- the meal session (pick_sessions.channel_id)
    source        TEXT NOT NULL,    -- the recipe being cooked — the plan's decision
    id            TEXT NOT NULL,
    step_id       INTEGER NOT NULL, -- which step of that recipe's stored reading
    started_at_ms INTEGER NOT NULL, -- the initiator's tap, in the shared timeline
    seconds       INTEGER NOT NULL, -- that step's duration, read from `recipes.steps`
    user_id       TEXT NOT NULL,    -- the telegram_user_id that started it

    -- `user_id` is deliberately NOT in the key, exactly as in `buy_checks`: a pot is on
    -- one hob, so the thing recorded is "this step is running" and the person is the
    -- attribution on it, not a second dimension of it. A second person tapping Start
    -- restarts the one timer (last writer wins, the way a shared kitchen works out
    -- loud) instead of adding a duplicate countdown for the same pot.
    PRIMARY KEY (channel_id, source, id, step_id)
);

-- No secondary index: timers are always read by (channel_id, source, id) — a prefix of
-- the primary key, already served by SQLite's automatic index on it.
