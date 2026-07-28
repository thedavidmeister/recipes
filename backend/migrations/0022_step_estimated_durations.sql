-- Estimated step durations (#158): every step carries a time, and the reading says
-- whether that time is the source's or the model's.
--
-- Measured against production before this change: 2,072 of 9,152 steps carried a
-- duration — 22.6%. Not one recipe in 790 was fully timed; 77 had no timed step at
-- all, and among the 713 that did, 92 claimed a total under ten minutes because
-- every untimed step contributed 0 to the critical path (a sixteen-step parcel
-- recipe read as "30 sec+"). The extraction was not at fault: the sources state
-- 2,124 explicit durations and the reading captured 2,072 of them. The sources are
-- simply silent, and the prompt told the model not to fill that silence.
--
-- The ruling: a missing time is a defect in our reading, never a property of the
-- dish. Every recipe takes some amount of time. So the reading now **estimates** an
-- unstated duration the way a cook does ("chop the onion" ≈ 90s), and
-- `StructuredStep` grows an `estimated` boolean beside `seconds` so the two kinds of
-- claim stay distinguishable — an all-stated recipe deserves more confidence than an
-- all-guessed one, and a display can only tell that truth if the structure carries
-- it. `recipe_core::step::validate` refuses a step with no duration, so no new
-- reading can reintroduce the hole; the equipment reading makes the same call
-- against an empty list (#81).
--
-- `total_seconds` is untouched. The model estimates, deterministic code still does
-- the arithmetic — the critical path just stops seeing zeros.
--
-- ## What this migration does
--
-- The shape lives in the JSON, not in a column, so there is nothing to ALTER. What
-- there is to do is make the readings already stored say out loud what they mean.
-- `estimated` deserializes as `false` when absent, which is correct for every row
-- here — the prompt that produced them forbade inventing a timer, and the measured
-- 2,072-vs-2,124 match confirms it was obeyed. Writing the flag in makes that a
-- recorded fact rather than a default anyone reading the table has to know about.
--
-- Applied to `step_structures` (the capture) and to `recipes.steps` (the derived
-- copy the browser reads directly, since there is no WASM and the frontend parses
-- nothing). Both are rewritten in place with json_group_array over json_each. Three
-- details the rebuild depends on:
--
--   * the `json()` wrapper, or the rebuilt elements are stored as quoted strings and
--     every reading in the corpus stops deserializing;
--   * `ORDER BY j.key`, because a step's `id` is its position in the array — a
--     reordered rebuild would silently rewire every dependency edge;
--   * `WHERE json_array_length(...) > 0`, because json_each over `[]` yields no rows,
--     so the subquery would return NULL and blank the column of every unread recipe.
--
-- Idempotent: json_set overwrites the key if a later run finds it already there.
--
-- ## What this migration deliberately does NOT do
--
-- It does not clear the readings. Re-reading the corpus is a deliberate, manual act
-- (clear the rows, re-run the worker) and never the routine path, so a migration —
-- which runs unattended on every boot — is exactly the wrong place for it. Until
-- that re-read is run by hand, the corpus keeps its untimed steps and its lower-bound
-- totals; they load, render and total exactly as before. `validate` gates the push,
-- not the load, so tightening what we accept invalidates nothing already captured.
UPDATE step_structures
SET structured = (
    SELECT json_group_array(json(v)) FROM (
        SELECT json_set(j.value, '$.estimated', json('false')) AS v
        FROM json_each(step_structures.structured) j
        ORDER BY j.key
    )
)
WHERE json_array_length(structured) > 0;

UPDATE recipes
SET steps = (
    SELECT json_group_array(json(v)) FROM (
        SELECT json_set(j.value, '$.estimated', json('false')) AS v
        FROM json_each(recipes.steps) j
        ORDER BY j.key
    )
)
WHERE json_array_length(steps) > 0;
