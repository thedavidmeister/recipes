#!/usr/bin/env node
/**
 * Run one read-only query against the live corpus and print the rows (#176/8).
 *
 * The design arguments in this repo keep turning on numbers — "92 recipes claim under
 * 10 minutes", "790 of 790 rows sat at their default", "223 runs say completed and 0
 * say failed". Every one of those is the difference between an argument and a guess,
 * and the difference between measuring and guessing has mostly been how much friction
 * stood in the way of a single `SELECT`. This removes it:
 *
 *     nix develop -c node frontend/scripts/corpus-query.mjs \
 *       "SELECT count(*) AS n FROM recipes WHERE equipment = '[]'"
 *
 * It is the ad-hoc peer of `sample-corpus.mjs`, which dumps a *fixed* set of rows for
 * the story fixtures; this answers a question you did not have yesterday. Same
 * credential, same place, same reason it lives under `frontend/` — that is where node
 * resolves `@libsql/client`, and the repo-root `.env` is where the **read-only** Turso
 * pair lives.
 *
 * Read-only twice over: the token itself cannot write, and this refuses anything that
 * is not a single `SELECT`/`PRAGMA`/`EXPLAIN`/`WITH`. The corpus has exactly one writer
 * per table (see CLAUDE.md), and an auditor is not one of them.
 */
import { createClient } from "@libsql/client";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;

const sql = process.argv.slice(2).join(" ").trim();
if (!sql) {
  console.error(
    'corpus-query: usage: node frontend/scripts/corpus-query.mjs "SELECT …"',
  );
  process.exit(1);
}

// One statement, and a reading one. Not a security boundary — the token is already
// read-only — but a refusal beats discovering the intent was wrong from the result,
// and it must FAIL CLOSED: `WITH doomed AS (…) DELETE …` and `PRAGMA user_version = 7`
// both write, so neither a WITH prefix nor PRAGMA can be waved through on its first
// word. Only SELECT and EXPLAIN lead a statement, and no write verb may appear
// anywhere in it — over-blocking a SELECT that merely quotes a word like "delete" is
// an acceptable cost for a tool whose whole claim is that it cannot write.
const WRITE_VERB =
  /\b(insert|update|delete|replace|drop|alter|create|pragma|vacuum|attach|detach|reindex)\b/i;
if (!/^\s*(select|explain|with)\b/i.test(sql) || WRITE_VERB.test(sql)) {
  console.error(
    `corpus-query: refusing ${
      JSON.stringify(sql.split(/\s+/)[0])
    } — this reads the ` +
      `corpus, it does not write it. Each table has exactly one writer and it is not ` +
      `this script.`,
  );
  process.exit(1);
}
if (sql.replace(/;\s*$/, "").includes(";")) {
  console.error("corpus-query: one statement at a time.");
  process.exit(1);
}

function env() {
  // Same four-line KEY=value reader as sample-corpus.mjs, and the same reason: this is
  // not worth a dependency. Root `.env`, because that is where the read-only pair lives.
  let text;
  try {
    text = readFileSync(join(ROOT, "../.env"), "utf8");
  } catch {
    return {};
  }
  const out = {};
  for (const line of text.split("\n")) {
    const m = /^\s*([A-Z0-9_]+)\s*=\s*(.*)$/.exec(line);
    if (m) out[m[1]] = m[2].trim().replace(/^["']|["']$/g, "");
  }
  return out;
}

const fromFile = env();
const url = process.env.PUBLIC_TURSO_URL ?? fromFile.PUBLIC_TURSO_URL;
const token = process.env.PUBLIC_TURSO_TOKEN ?? fromFile.PUBLIC_TURSO_TOKEN;

if (!url || !token) {
  console.error(
    "corpus-query: PUBLIC_TURSO_URL / PUBLIC_TURSO_TOKEN are not set — put the " +
      "read-only pair in the repo-root .env (see .env.example). Do not substitute " +
      "the write token, and do not guess an answer this was meant to measure.",
  );
  process.exit(1);
}

const rows = (await createClient({ url, authToken: token }).execute(sql)).rows;

// BigInt is what libsql hands back for INTEGER, and JSON.stringify throws on it.
// Number() silently rounds past 2^53, and ids/run_ids are exactly the columns that
// could get there — so anything unsafe is printed as a string rather than a lie.
console.log(
  JSON.stringify(
    rows.map((r) => Object.fromEntries(Object.entries(r))),
    (_k, v) =>
      typeof v === "bigint"
        ? (v <= BigInt(Number.MAX_SAFE_INTEGER) &&
            v >= -BigInt(Number.MAX_SAFE_INTEGER)
          ? Number(v)
          : v.toString())
        : v,
    2,
  ),
);
console.error(`corpus-query: ${rows.length} row(s)`);
