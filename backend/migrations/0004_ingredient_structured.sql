-- The LLM's structured reading of an ingredient line (#11), keyed by the raw
-- line so it dedups across recipes and — crucially — survives a `derive`.
--
-- This is the third corpus table, and a *peer* of raw_imports rather than a
-- stage-private silo: `enrich` writes it (the only networked step, via Haiku),
-- and `derive` reads it to reattach `structured` onto each recipe's ingredients
-- with no LLM call. That is why `derive` stays offline — both of its inputs
-- (raw_imports, this) are already persisted. Raw stays the source of truth in
-- raw_imports/recipes; this is the enrichment half (parse-but-preserve).
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
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (name, measure)
);
