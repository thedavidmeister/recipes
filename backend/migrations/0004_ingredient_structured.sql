-- The LLM's structured reading of an ingredient line (#11), keyed by the raw
-- line so it dedups across recipes and — crucially — survives a `derive`.
--
-- This is the third corpus table, and a *peer of raw_imports*, not of recipes.
-- `recipes = derive(raw_imports)` is a pure, deterministic derivation. A reading
-- is NOT: a model is non-deterministic (same line, re-run, can differ) and drifts
-- over time (an endpoint's model changes; you switch it). So a reading is a
-- **captured artifact at a point in time**, like a fetched page — which is why it
-- lives on the source side, written once by `enrich` (the only networked step)
-- and read by `derive` to reattach `structured` with no LLM call. Deriving stays
-- offline because both of its inputs (raw_imports, this) are already persisted.
--
-- The cache is therefore not "memoization of a pure function" — it exists BECAUSE
-- the extraction is not reproducible: capture a reading once and every rebuild
-- reuses that exact one instead of re-rolling a non-deterministic, drifting model
-- (and re-paying). Each row records its provenance — which `model`, and `when` —
-- so drift is auditable and a deliberate re-capture (a better model) can target
-- it. Re-capturing is an explicit act, never a silent side effect of a derive.
--
-- Line-keyed, not recipe-keyed, so a normalizer fix that re-splits one recipe's
-- lines only re-enriches the lines that actually changed, and a line shared
-- across recipes ("salt" / "to taste") is read once.
CREATE TABLE IF NOT EXISTS ingredient_structured (
    -- The raw line, verbatim, as the cache key. `measure` is '' (never NULL)
    -- when the source gave none, because SQLite treats NULLs in a key as
    -- distinct — a NULL here would defeat the dedup the key exists for.
    name       TEXT NOT NULL,
    measure    TEXT NOT NULL DEFAULT '',
    -- The StructuredMeasure as JSON — exactly what lands in Ingredient.structured.
    structured TEXT NOT NULL,
    -- Provenance: which model produced this reading, and when it was captured.
    -- Present because the reading is non-deterministic and model-versioned — this
    -- is what makes drift visible and a targeted re-capture possible.
    model      TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (name, measure)
);
