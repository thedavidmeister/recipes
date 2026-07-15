-- The raw side of the corpus: each recipe's payload exactly as its source gave
-- it, kept so `recipes` can be rebuilt without touching upstream.
--
-- Keyed by the recipe, not by the fetch. Raw is not an archive of everything we
-- downloaded — we only want recipes. A category listing is a taxonomy, not a
-- recipe, and a browse response of partials never reaches the corpus, so neither
-- is stored. One recipe therefore has exactly one raw row however many responses
-- happened to mention it.
--
-- `raw` is a minimal document its own adapter can re-normalize (for TheMealDB,
-- the meal wrapped in the `{"meals":[…]}` envelope it arrived in), so deriving
-- runs the same code path as ingesting, not a special one.
--
-- Cold by design: `recipes` carries everything search/browse render, so list
-- queries never touch this table and never load a payload.
CREATE TABLE IF NOT EXISTS raw_imports (
    source       TEXT    NOT NULL,
    id           TEXT    NOT NULL,
    raw          TEXT    NOT NULL,
    content_type TEXT,
    -- The URL it came from. Adapters take a URL (schema.org derives a recipe's
    -- id and source_url from it), so deriving needs it.
    source_url   TEXT,
    fetched_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (source, id)
);

-- Derive walks by source, so fixing one adapter can replay only its rows.
CREATE INDEX IF NOT EXISTS raw_imports_source_idx ON raw_imports (source);
