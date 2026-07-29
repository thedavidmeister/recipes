-- The meal-time reading (#191): when each dish is actually eaten, so a plan for a
-- meal can deal that meal.
--
-- The fifth enrichment, and its own table like the four before it (0004 ingredients,
-- 0008 steps, 0014 equipment, 0023 nutrition) — never a row in a generic (kind, json)
-- container. Its shape is decided here, which is the whole reason CLAUDE.md says a
-- future enrichment gets a migration.
--
-- ## Numbering: why 24
--
-- `db.rs` applies migrations by MAX(version), so a number at or below one already
-- applied on production never runs at all. Production `_migrations` is at 23; 20 is a
-- hole that 21, 22 and 23 have already overtaken and which can never be filled. Taking
-- the next number above everything, including anything unmerged on another branch, is
-- the only choice that is safe whichever deploys first.
--
-- ## Why this reading has to exist
--
-- A plan asks "which meal", stores the answer, and shows it in the heading. #188 made
-- that answer narrow the deck as far as *stated* data allows — it drops the 264 dishes
-- the corpus says accompany a meal (`Dessert` 166, `Side` 84, `Starter` 14) — but that
-- exclusion is identical for all four meal words, because the corpus has no `Lunch`,
-- `Dinner` or `Snack` category at all and `Breakfast` is 19 of 790. Filtering strictly
-- on stated category today would give Breakfast 19 recipes and the other three **zero**.
--
-- So nothing the corpus states can tell breakfast from dinner. This table is the first
-- thing that can.
--
-- ## A set of sittings, and it may not be empty
--
-- `sittings` is a JSON array of the closed vocabulary — breakfast, lunch, dinner,
-- snack — the same four words a plan's `pick_sessions.meal_type` carries, and by then
-- the same Rust type (`recipe_core::meal::Sitting`), so a bound is set membership and
-- never a mapping that could drift:
--
--     ["lunch","dinner"]      -- a chicken curry
--     ["breakfast"]           -- pancakes
--     ["breakfast","snack"]   -- toast
--     ["dinner"]              -- a roast
--
-- A **set**, not a label, because most dishes genuinely suit more than one sitting; a
-- single word would be wrong on its face and would make the filter it exists for
-- useless. Stored in vocabulary order so one fact is one row (`recipe_core::meal::
-- canonical`), and never empty: every dish is eaten at some time, so an empty set is a
-- failed reading rather than a fact about the food (#158's ruling on durations, #162's
-- on servings). The app refuses one on the way in, which is what lets an empty array
-- mean **unread** everywhere else.
--
-- Keyed per `(source, id)` like the other four: when a dish is eaten is a fact about
-- the dish, and the whole cascade — the pull's left join, the `run_id` guard, the
-- targeted re-derive — is keyed that way.
CREATE TABLE IF NOT EXISTS meal_time_structures (
    source     TEXT NOT NULL,
    id         TEXT NOT NULL,
    sittings   TEXT NOT NULL,
    model      TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    run_id     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (source, id)
);

-- The derived view carries the reading alongside the recipe, the way `steps`,
-- `equipment` and `nutrition` do, because the walk filters on it once per request and
-- joining the capture table per walk would be a second query for a column `recipes`
-- already has a row for.
--
-- `'[]'` until the worker has read it — degrade-not-die, and the same default the other
-- readings take. It reads as **unread**, never as "eaten at no sitting": the reading
-- refuses an empty set on the way in, so `[]` can only mean nobody has read this yet,
-- and `recipe_core::meal::fit` returns `Unread` for it and does not restrict the deck.
--
-- `recipes::upsert` names this column in its INSERT *and* in its ON CONFLICT SET. That
-- is #161: `equipment` was correctly produced and reattached at derive for months while
-- the sole writer of this table never listed it, so all 790 rows sat at the `'[]'`
-- default and nobody could tell until a feature needed the data.
ALTER TABLE recipes ADD COLUMN sittings TEXT NOT NULL DEFAULT '[]';
