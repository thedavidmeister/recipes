-- The structured-ingredients enrichment of a recipe (#11): the LLM's reading of
-- its ingredient lines, ONE row per recipe (source, id), stored beside the raw.
--
-- A dedicated table for this one enrichment — not a generic (kind, json) container.
-- Enrichments are developer-added over time (each is a new extractor plus a way to
-- apply it), so each is a deliberate, modelled thing: a future 'nutrition' or
-- 'allergens' enrichment gets its OWN table with its own columns, added by its own
-- migration — which is the right place to decide its shape, not an untyped blob.
--
-- Per-recipe, not per-line: it matches #11's "once per recipe at write time", and
-- keeps the raw → enrich → derive cascade a clean per-(source, id) chain — no
-- line→recipe fan-out. `structured` is the JSON array of readings aligned to the
-- recipe's ingredient order; `derive` zips it back on, guarding on count so a
-- reading left over from a since-changed raw simply doesn't attach.
--
-- It is a *capture*, not a derivation: a model is non-deterministic and drifts, so
-- `model` + `created_at` record which model read this recipe and when — provenance
-- for a re-snapshot decision (`enrich --refresh`), the way raw_imports has
-- `fetched_at`.
CREATE TABLE IF NOT EXISTS ingredient_structures (
    source     TEXT NOT NULL,
    id         TEXT NOT NULL,
    structured TEXT NOT NULL,
    model      TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (source, id)
);
