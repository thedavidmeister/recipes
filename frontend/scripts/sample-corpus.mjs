#!/usr/bin/env node
/**
 * Refresh `src/lib/corpus-sample.json` — the committed sample of real corpus rows
 * the story fixtures are built from (#157).
 *
 * Story fixtures mirror real source records (CLAUDE.md), and that has to include
 * field *values*, not just ids and images. A hand-typed value drifts silently and
 * then code gets fitted to it: #84 shipped an `UntimedRecipe` story using a recipe
 * that has a time in production, and invented round `total_seconds` that made an
 * hours-formatting rule look correct while it rendered a real 9000s as "150 min+".
 * The same class produced this script's reason to exist — `recipeSteps()` claimed a
 * 2160s critical path for a recipe the corpus stores at 1380s.
 *
 * So the fixtures do not hand-type corpus values any more: they read them out of a
 * sample dumped straight from Turso by this script. Refreshing is a deliberate act
 * (like `visual:update`), and the diff is the drift — never silent.
 *
 * This lives under `frontend/` so node resolves `@libsql/client`, and reads the
 * repo-root `.env` where `PUBLIC_TURSO_URL` / `PUBLIC_TURSO_TOKEN` (read-only) live.
 *
 *     nix develop -c node frontend/scripts/sample-corpus.mjs
 *
 * It also prints the corpus aggregates, because `healthStats()` in `fixtures.ts` is
 * an aggregate snapshot rather than a record — nothing stable to derive from — so it
 * stays hand-written against the reading this prints, with the date in its comment.
 */
import { createClient } from "@libsql/client";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const OUT = join(ROOT, "src/lib/corpus-sample.json");

/**
 * The recipes the stories name, and why each one is in the sample. Every id here is
 * a real TheMealDB record; adding one means adding a row a story actually renders,
 * not padding the file.
 */
const WANTED = [
  [
    "themealdb",
    "52795",
    "the base recipe — buy, cook, and the walk's first stop",
  ],
  ["themealdb", "52820", "walk stop 2"],
  ["themealdb", "52772", "walk stop 3"],
  ["themealdb", "52827", "walk stop 4 — the long one (past the hour)"],
  ["themealdb", "52874", "walk stop 5"],
  [
    "themealdb",
    "53239",
    "the unread card — total_seconds is null in the corpus",
  ],
  [
    "themealdb",
    "53287",
    "the longest title in the corpus, for the wrapping story",
  ],
];

function env() {
  // No dotenv dependency: this is `KEY=value` lines, and a parser for that is four
  // lines. Root `.env`, because that is where the read-only Turso pair lives.
  const text = readFileSync(join(ROOT, "../.env"), "utf8");
  const out = {};
  for (const line of text.split("\n")) {
    const m = /^\s*([A-Z0-9_]+)\s*=\s*(.*)$/.exec(line);
    if (m) out[m[1]] = m[2].trim().replace(/^["']|["']$/g, "");
  }
  return out;
}

const { PUBLIC_TURSO_URL, PUBLIC_TURSO_TOKEN } = env();
if (!PUBLIC_TURSO_URL || !PUBLIC_TURSO_TOKEN) {
  console.error(
    "sample-corpus: PUBLIC_TURSO_URL / PUBLIC_TURSO_TOKEN missing from the repo-root .env",
  );
  process.exit(1);
}

const db = createClient({
  url: PUBLIC_TURSO_URL,
  authToken: PUBLIC_TURSO_TOKEN,
});

// The app-facing columns of `recipes` — the row as the app reads it. `fetched_at`
// and `run_id` are pipeline bookkeeping no surface renders, so they are left out;
// nothing else is, because trimming a row to what a story happens to use today is
// how a fixture starts editorialising again.
const COLUMNS =
  "source, id, title, image, category, area, tags, ingredients, instructions, steps, equipment, source_url, video_url, total_seconds, kcal, kcal_complete, servings";

const rows = [];
for (const [source, id, why] of WANTED) {
  const rs = await db.execute({
    sql: `SELECT ${COLUMNS} FROM recipes WHERE source = ? AND id = ?`,
    args: [source, id],
  });
  const row = rs.rows[0];
  if (!row) {
    console.error(`sample-corpus: ${source}/${id} is not in the corpus`);
    process.exit(1);
  }
  // `tags`, `ingredients`, `steps` and `equipment` are stored as JSON text; the
  // sample holds them as JSON so the fixtures need no parsing step of their own.
  rows.push({
    why,
    source: row.source,
    id: row.id,
    title: row.title,
    image: row.image,
    category: row.category,
    area: row.area,
    tags: JSON.parse(row.tags),
    ingredients: JSON.parse(row.ingredients),
    instructions: row.instructions,
    steps: JSON.parse(row.steps),
    equipment: JSON.parse(row.equipment),
    source_url: row.source_url,
    video_url: row.video_url,
    total_seconds: row.total_seconds === null
      ? null
      : Number(row.total_seconds),
    // The nutrition reading's three app-facing columns (#162). They are here for the
    // reason this whole file exists: the pick card's calorie badge is per serving, so
    // a fixture that carried a total without the servings it is divided by — or
    // without `kcal_complete` — would render a claim the corpus never made. NULL
    // stays NULL: unread is a state the badge has to show (as nothing at all), and
    // the sample is where it comes from.
    kcal: row.kcal === null ? null : Number(row.kcal),
    kcal_complete: Number(row.kcal_complete) !== 0,
    servings: row.servings === null ? null : Number(row.servings),
  });
}

const sampled_at = new Date().toISOString().slice(0, 10);
writeFileSync(
  OUT,
  `${JSON.stringify({ sampled_at, recipes: rows }, null, 2)}\n`,
  "utf8",
);
console.log(
  `sample-corpus: wrote ${rows.length} rows to src/lib/corpus-sample.json`,
);

// The aggregates `healthStats()` mirrors. Printed, not written: they move every few
// hours (the ingest schedule), so pinning them into the sample would redraw the
// health baselines on every refresh of an unrelated recipe row.
const one = async (sql) => Number((await db.execute(sql)).rows[0][0]);
const recent = await db.execute(
  "SELECT id, kind, status, started_at, finished_at FROM runs ORDER BY id DESC LIMIT 5",
);
console.log(
  "\nhealthStats() reading — paste into fixtures.ts with today's date:",
);
console.log(`  recipes  ${await one("SELECT count(*) FROM recipes")}`);
console.log(`  raw      ${await one("SELECT count(*) FROM raw_imports")}`);
console.log(
  `  enriched ${await one("SELECT count(*) FROM ingredient_structures")}`,
);
console.log(
  `  running  ${await one(
    "SELECT count(*) FROM runs WHERE status = 'running'",
  )}`,
);
console.log(
  `  by_model ${
    JSON.stringify(
      (
        await db.execute(
          "SELECT model, count(*) AS c FROM ingredient_structures GROUP BY model ORDER BY c DESC",
        )
      ).rows.map((r) => ({ model: r.model, count: Number(r.c) })),
    )
  }`,
);
console.log(
  `  recent_runs ${
    JSON.stringify(
      recent.rows.map((r) => ({
        id: Number(r.id),
        kind: r.kind,
        status: r.status,
        started_at: Number(r.started_at),
        finished_at: r.finished_at === null ? null : Number(r.finished_at),
      })),
    )
  }`,
);
