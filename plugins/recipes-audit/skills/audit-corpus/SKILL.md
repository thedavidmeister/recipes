---
name: audit-corpus
description: >-
  Audit the recipes tree against the live corpus — the periodic sweep, not a diff
  review. Measure what production actually holds and compare it to what the code, the
  fixtures, the comments and the design arguments claim: derived columns that are
  populated in the schema but empty in the rows, statuses nothing ever writes, story
  fixtures that have drifted from the records they mirror, migrations below the applied
  floor, and any number stated without a query behind it. Needs the frontend's
  read-only Turso credential. For reviewing a change, use `audit-change`.
---

# Audit the recipes tree against production

`audit-change` reviews a diff against the intent it claims. This is the other
oracle: **the corpus itself**. It answers questions no branch and no test can,
because the only place the answer lives is the live database — how many rows
actually carry the reading, which statuses were ever written, what the highest
applied migration is.

Run it as a periodic sweep, or whenever someone is about to make an argument
that turns on a number.

The premise, from #176: _"these are the difference between a design argument and
a guess."_ Querying the corpus is the **default move here, not an extra step.**

## The credential

One pair, read-only, in the repo-root `.env` (gitignored):

- `PUBLIC_TURSO_URL` — the Turso database URL.
- `PUBLIC_TURSO_TOKEN` — the **read-only** token. This is the same credential
  the browser gets, and it is the right one: the frontend reads Turso directly,
  so a token that can read the whole corpus and write none of it already exists
  for exactly this purpose.

`.env.example` documents both. If they are absent:

- **Say so and stop.** Report which checks you could not run, and why.
- **Do not go looking.** Not in the environment, not in Render, not in another
  repo, not in shell history.
- **Never substitute the write token** (`TURSO_AUTH_TOKEN`). Each table in this
  corpus has exactly one writer (see CLAUDE.md) and an auditor is not one of
  them. A read-only credential is a design property, not an inconvenience.
- **Never guess the number.** An unmeasured claim is precisely the defect this
  skill exists to find; producing one yourself is the worst possible outcome.

## Measuring

```sh
nix develop -c node frontend/scripts/corpus-query.mjs "SELECT count(*) AS n FROM recipes"
```

`corpus-query.mjs` refuses anything that is not a single
`SELECT`/`PRAGMA`/`EXPLAIN`/ `WITH`, reads the pair from the repo-root `.env`,
and prints rows as JSON. Its sibling `sample-corpus.mjs` refreshes the committed
fixture sample and prints the aggregates `healthStats()` mirrors — use that one
when the finding is "the fixtures have drifted".

Quote the **query and its result** in every finding. A number without the query
behind it is the thing you are hunting.

## The checks

### 1. A derived column that is populated in the schema and empty in the rows

The #161 shape, measured: migration 0014 added `recipes.equipment`, `derive`
computed it, `upsert` never named it, and **790 of 790 rows** read `'[]'` for
months.

`backend/src/recipes.rs` now has tests that fail if `upsert` stops naming a
column, so the _writer_ is gated. The **rows are not**, and they are a separate
fact: a column fixed today stays stale on every row written before the fix until
a re-derive actually runs. That gap is invisible from the repo.

For every non-key column of `recipes`, ask what fraction of rows sit at the
default:

```sh
nix develop -c node frontend/scripts/corpus-query.mjs "
  SELECT count(*) AS rows,
         sum(equipment = '[]')  AS no_equipment,
         sum(nutrition = '[]')  AS no_nutrition,
         sum(steps = '[]')      AS no_steps,
         sum(total_seconds IS NULL) AS no_total,
         sum(kcal IS NULL)      AS no_kcal,
         sum(fully_timed = 0)   AS not_fully_timed
  FROM recipes"
```

**Read it against the capture tables, not in isolation.** A column empty on
every row is a bug only if the reading exists to fill it — that is what made
#161 damning (790/790 had a reading in `equipment_structures` and 790/790 rows
read `'[]'`), and what makes an un-run enrichment merely pending:

```sh
nix develop -c node frontend/scripts/corpus-query.mjs "
  SELECT (SELECT count(*) FROM recipes) AS recipes,
         (SELECT count(*) FROM ingredient_structures) AS ingredient_readings,
         (SELECT count(*) FROM step_structures)       AS step_readings,
         (SELECT count(*) FROM equipment_structures)  AS equipment_readings,
         (SELECT count(*) FROM nutrition_structures)  AS nutrition_readings"
```

A reading count far above the populated-column count is the finding. Report it
as the pair of numbers.

### 2. A status or variant nothing has ever written

`runs::FAILED` was reachable only from the CLI, so every service path wrote
`COMPLETED`: **223 runs said completed and 0 said failed** (#174). The code
looked like it had two outcomes. Production had one.

```sh
nix develop -c node frontend/scripts/corpus-query.mjs "
  SELECT kind, status, count(*) AS n FROM runs GROUP BY kind, status ORDER BY n DESC"
```

Then read it **both ways**, exactly as `audit-change` item 6 does:

- A variant the enum declares with **0 rows** — is there a reachable path that
  writes it, or is the code describing an outcome that cannot happen?
- A value in the rows that the **surfaces do not distinguish** — a status
  present in production that `HealthDashboard.badgeCls` (or any renderer) folds
  into a fallthrough is being displayed as something it is not.

The same question applies to any low-cardinality column:
`ingredient_structures.model`, `recipes.source`, `recipes.category`. Group by it
and look for the value that is 0, and the value nothing reads.

### 3. Migrations below the applied floor

`db.rs` applies by `MAX(version)`, so a migration numbered below the highest
already applied **never runs**, silently and permanently.
`migration_ledger_is_well_formed` pins the ledger's shape, but the floor lives
only here:

```sh
nix develop -c node frontend/scripts/corpus-query.mjs "
  SELECT max(version) AS applied_floor, count(*) AS applied FROM _migrations"
```

Compare against `MIGRATIONS` in `backend/src/db.rs`:

- Any registered version **≤ the floor that is not in `_migrations`** has
  already been skipped forever. That is a live defect, not a warning — the
  column it was meant to add does not exist in production.
- Any number in `RESERVED` at or below the floor is **burnt**: it can never be
  filled, and its comment should say so.
- A migration on an unmerged branch numbered at or below the floor is dead on
  arrival.

### 4. Fixtures that have drifted from the records they mirror

Story fixtures mirror **real** source records (CLAUDE.md), and that includes
field _values_ — #84 shipped a story using a recipe that has a time in
production, with an invented round `total_seconds` that made a formatting rule
look right while a real 9000s rendered as "150 min+".

`fixtures.ts` reads `corpus-sample.json`, and `row()` throws on an id that is
not in it — so the ids are self-checking against the **snapshot**. What this
skill adds is whether the snapshot still matches **reality**, and whether
anything bypassed it:

- Re-run `sample-corpus.mjs`. **The diff is the drift.** No diff means the
  fixtures are still true; a diff means a story has been rendering a recipe that
  has since changed.
- Grep the fixtures and stories for any recipe id or field value that does
  **not** go through `row()`/`card()` — a hand-typed card, a hand-typed
  `total_seconds`, an id built inline. Each one is either a documented exception
  or drift, and only a query can tell you which:

  ```sh
  nix develop -c node frontend/scripts/corpus-query.mjs "
    SELECT source, id, title, total_seconds, fully_timed
    FROM recipes WHERE source = 'themealdb' AND id = '<the hand-typed id>'"
  ```

  If the row exists and the values match, the fixture is honest and the fix is
  to add it to `WANTED` in `sample-corpus.mjs` so it stays honest. If it does
  not, it is an invented record rendering a recipe that does not exist.

- Check that a fixture asserting an **absence** still has one — the "no
  estimate" card is only that story while `total_seconds IS NULL` in the corpus.

### 5. Claims nobody measured

Sweep the tree — code comments, `CLAUDE.md`, `README.md`, open issues, PR bodies
— for statements of fact about production. They read like: "92 recipes claim
under 10 minutes", "only 1 kitchen, holding 0 equipment and 0 pantry", "2,072 of
9,152 steps are timed", "790 of 790 rows".

For each: **run the query and compare.** Report three outcomes distinctly,
because they need different fixes:

- **Still true** — say so with the current number; a claim with a date beside it
  is worth more than one without.
- **Drifted** — the number has moved. Update the claim, or, if a design decision
  rests on it, flag that the decision's premise has changed.
- **Never was true** — the worst case, and the reason for this check. Say which
  argument was built on it.

A claim that carries a date and a query is a measurement. A claim that carries
neither is a guess, and should be labelled one even when it turns out to be
right.

## Reporting

Per finding: **the claim**, **the query**, **the result**, **the fix**. Rate by
production impact — a stale column that every surface reads outranks a comment
whose number moved by three.

State plainly which checks you ran and which you skipped for want of the
credential. Finish with the date and the headline aggregates you measured, so
the next sweep has a baseline to diff against.

## Do / don't

- **Do** put a query behind every number, and quote both.
- **Do** read a column's emptiness against its capture table before calling it a
  bug.
- **Do** re-run `sample-corpus.mjs` and read the diff as the drift signal.
- **Don't** guess a number, round one, or carry one forward from an old comment.
- **Don't** write to the corpus. Every table has exactly one writer and it is
  not you.
- **Don't** go looking for credentials, and never use the write token.
- **Don't** re-derive what `audit-change` and CI already cover — this skill
  exists for the questions only production can answer.
