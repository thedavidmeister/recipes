-- Nutrition reading (#162): roughly what a recipe costs you, in kcal.
--
-- The fourth enrichment, and its own table like the three before it
-- (0004 ingredients, 0008 steps, 0014 equipment) — never a row in a generic
-- (kind, json) container. Its shape is decided here, which is the whole reason
-- CLAUDE.md says a future enrichment gets a migration.
--
-- ## Numbering: why 23
--
-- `db.rs` applies migrations by MAX(version), so a number below one already applied on
-- production never runs at all. 22 is the highest merged; 20 is a hole reserved for
-- work in flight that 21 and 22 have already overtaken and which must itself be
-- renumbered. Taking the next number above everything — rather than filling the hole
-- — is the only choice that is safe against whatever deploys first.
--
-- ## What is captured, and what is computed
--
-- `structured` is a JSON array of FoodEnergy, one entry per ingredient in the recipe's
-- ingredient order — the same alignment as `ingredient_structures`, and what lets the
-- push refuse a reading that has drifted from the recipe.
--
-- Each entry holds facts about the **food**, never about this recipe's quantity:
--
--     { "kcal_per_100g": 364.0, "grams_per_unit": 125.0 }
--
-- `kcal_per_100g` is the energy density; `grams_per_unit` is the mass of one of the
-- line's own unit (one cup, one clove, one egg) and is **omitted** whenever the unit is
-- a mass unit the converter already knows. Neither number grows with the recipe, which
-- is the point: the model is never asked to add up. `recipe_core::nutrition` multiplies
-- by the quantity the ingredient reading already captured and sums, so a scaled recipe
-- (`StructuredMeasure::scaled`) comes out right with no nutrition-specific code.
--
-- ## servings is a column, not a field in the JSON
--
-- No source in the corpus carries a yield, so how many people a recipe feeds is a
-- reading like the rest. It is a **different kind of claim** from the per-ingredient
-- array — one number about the whole dish, inferred from the title, the method and the
-- quantities — so it gets its own column rather than hiding inside `structured`, and
-- is queryable on its own.
--
-- It is captured at all because a bare total is ambiguous exactly where it matters:
-- 2,400 kcal is a reasonable tray of lasagne and an absurd plate of it. Per the #158
-- ruling, a recipe nobody has written a serving count for still feeds a definite number
-- of people — the gap is in our reading, not in the dish.
CREATE TABLE IF NOT EXISTS nutrition_structures (
    source     TEXT NOT NULL,
    id         TEXT NOT NULL,
    structured TEXT NOT NULL,
    servings   INTEGER NOT NULL,
    model      TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    run_id     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (source, id)
);

-- The derived view carries the reading alongside the recipe, the way `steps` and
-- `equipment` do. `[]` until the worker has read it — degrade-not-die.
--
-- It is stored here and not only in the capture table so that `recipes::upsert` can
-- compute `kcal` from the very column it writes beside it, in one statement. A total
-- computed somewhere else and passed in could drift from the reading it claims to
-- summarise; this cannot.
ALTER TABLE recipes ADD COLUMN nutrition TEXT NOT NULL DEFAULT '[]';

-- How many people the recipe feeds, as read. NULL until read — and NULL, not 1,
-- because "we have not read this" and "this feeds one person" are different facts and
-- the display owes the reader the difference.
ALTER TABLE recipes ADD COLUMN servings INTEGER;

-- Total kcal for the **whole** recipe, computed by `recipe_core::nutrition` over the
-- `nutrition` column and the structured measures in `ingredients`, in the same
-- `recipes::upsert` write. Per-serving is `kcal / servings` — a division, not a
-- definition, so the surface does it rather than a fourth stored column that could
-- disagree with the two it came from.
--
-- Whole kcal, deliberately. The estimate is not accurate to a calorie and storing a
-- float would invite a display that renders one. NULL — not 0 — when nothing could be
-- counted: an unread recipe, or one whose every line is "to taste". A 0 would render as
-- a dish with no calories, which is never true of food; absence reads as unknown, which
-- is the honest state (#158) and the same call `total_seconds` makes.
ALTER TABLE recipes ADD COLUMN kcal INTEGER;

-- Is that total complete, or only a floor? The peer of `fully_timed` (0022), and it
-- exists for the same reason: the browser reads Turso directly and cannot run
-- recipe-core, so "is this number complete" is decided once, server-side, beside the
-- number itself, instead of a second definition in JS.
--
-- `1` when every ingredient line that stated a number was turned into grams. `0` when
-- at least one was not — a unit no table knows and no reading covered — so the total
-- can only be too low.
--
-- A line with **no** number ("salt, to taste") does not clear this flag. It is not a
-- failed reading; it is a line that never had a quantity, and since nearly every recipe
-- has a pinch of something, counting those would leave the flag permanently 0 and say
-- nothing true. The unmeasurable tail — "to taste", oil left in the pan — is a property
-- of every calorie estimate ever made and is hedged once at the surface (#84), not
-- per-recipe.
--
-- NOT NULL DEFAULT 0 like `fully_timed`, and for the same reason: there is no
-- meaningful third state (an unread recipe's `kcal` is NULL anyway), and the default
-- backfills every existing row correctly, since none of them has been read.
ALTER TABLE recipes ADD COLUMN kcal_complete INTEGER NOT NULL DEFAULT 0;
