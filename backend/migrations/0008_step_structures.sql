-- Step reading (#74/#75/#76): a model's structured reading of a recipe's method,
-- one row per recipe. A peer of ingredient_structures (#11) — its own table, per
-- "each enrichment its own table", not an untyped blob.
--
-- `structured` is a JSON array of StructuredStep: the method segmented into a DAG
-- of steps, each with a kind (prep | cook), an optional timer duration, and the
-- ids of the steps that must finish before it (its `after` edges). Parallel vs
-- sequential is derived from those edges, not stored. The model both segments the
-- instructions and maps the dependencies; deterministic code does no arithmetic
-- here (unlike measures), so this is a pure capture.
--
-- run_id is a column from the start (unlike the older tables, which 0005 ALTERed
-- in) so a stale or partial run can't clobber a newer reading.
CREATE TABLE IF NOT EXISTS step_structures (
    source     TEXT NOT NULL,
    id         TEXT NOT NULL,
    structured TEXT NOT NULL,
    model      TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    run_id     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (source, id)
);

-- The derived view carries the reading alongside the raw method, the way each
-- ingredient's `structured` rides inside the `ingredients` JSON. `[]` until the
-- step-reading worker has read the recipe (degrade-not-die).
ALTER TABLE recipes ADD COLUMN steps TEXT NOT NULL DEFAULT '[]';
