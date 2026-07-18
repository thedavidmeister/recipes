-- Run generation (#11 write-path hardening): make corpus writes ordered so
-- concurrent or partial runs cannot clobber each other.
--
-- The write path was latest-writer-by-execution-order — a lost update the moment
-- two runs overlap (a manual `enrich`/`derive` against the scheduled
-- `/api/ingest`, an `enrich --refresh` against a routine run) or one dies
-- mid-flight and commits late. Wall-clock timestamps cannot arbitrate that:
-- Render and a CLI box have clock skew.
--
-- Every invocation opens a row here at start and closes it at end. `id` is
-- DB-assigned and monotonic, so it is a *total order free of clock skew* — which
-- is why it, not `started_at`, decides who wins a write.
CREATE TABLE IF NOT EXISTS runs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    -- 'ingest' (the sync→enrich→derive pipeline), 'enrich', 'derive', 'refresh'.
    kind        TEXT NOT NULL,
    started_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    -- NULL while running — a row still NULL long after `started_at` is a run that
    -- died partway, which is exactly the state this table makes visible.
    finished_at INTEGER,
    status      TEXT NOT NULL DEFAULT 'running' -- 'running' | 'completed' | 'failed'
);

-- Stamp every corpus row with the run that last wrote it. Each writer's upsert
-- guards with `WHERE excluded.run_id >= <table>.run_id`, so only an equal-or-newer
-- run overwrites: a stale/partial older run can never clobber a newer one. `>=`,
-- not `>`, so a run re-writing its own row (the same recipe named by two catalog
-- responses in one sync) still applies; cross-run is always strict because ids
-- are unique. Default 0 for rows written before this existed, so any real run
-- (id >= 1) supersedes them.
--
-- One writer per table — raw_imports ← sync, ingredient_structures ← enrich,
-- recipes ← derive — so these three are the only write paths the guard covers.
ALTER TABLE raw_imports ADD COLUMN run_id INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ingredient_structures ADD COLUMN run_id INTEGER NOT NULL DEFAULT 0;
ALTER TABLE recipes ADD COLUMN run_id INTEGER NOT NULL DEFAULT 0;
