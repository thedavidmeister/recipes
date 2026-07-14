-- The recipe corpus. Rows are normalized recipes from any source; the
-- write-gateway upserts on (source, id). `tags` and `ingredients` are JSON
-- (a string array, and an array of {name, measure} respectively). Browse /
-- listing rows may be partially populated (empty detail).
CREATE TABLE IF NOT EXISTS recipes (
    source       TEXT    NOT NULL,
    id           TEXT    NOT NULL,
    title        TEXT    NOT NULL,
    image        TEXT,
    category     TEXT,
    area         TEXT,
    tags         TEXT    NOT NULL DEFAULT '[]',
    ingredients  TEXT    NOT NULL DEFAULT '[]',
    instructions TEXT    NOT NULL DEFAULT '',
    source_url   TEXT,
    video_url    TEXT,
    fetched_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (source, id)
);

CREATE INDEX IF NOT EXISTS recipes_title_idx ON recipes (title);
CREATE INDEX IF NOT EXISTS recipes_category_idx ON recipes (category);
CREATE INDEX IF NOT EXISTS recipes_area_idx ON recipes (area);
