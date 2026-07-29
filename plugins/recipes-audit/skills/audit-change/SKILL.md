---
name: audit-change
description: >-
  Audit a change to the recipes repo — a PR, a branch, or a working diff — for the
  defects CI cannot catch: code that is type-correct, lint-clean, fully tested, green,
  and still describes something that is not true. Use when reviewing a PR here, before
  merging, or when asked to check a change. It reviews a diff against the intent it
  claims and against the tree that moved underneath it; for measuring the tree against
  production, use `audit-corpus` instead.
---

# Audit a change to the recipes repo

This repo's CI is strong: `lint:design`, `lint:stories`, `lint:env`, a visual
fence that pixel-diffs every story, `svelte-check`, `cargo clippy -D warnings`,
and a Rust suite that pins the migration ledger and every column of `recipes`.
**Do not re-check what those prove.** A skill that re-runs a gate is waste.

What gets through them is one specific class: a change that is **correct in
every mechanical sense and still asserts something false**. A fixture that ticks
the wrong ingredient. A column computed and thrown away. A status no path can
write. A claim in a PR body nobody measured. Every check below is one that has
actually shipped here — the issue numbers are real and worth reading when a
check fires.

## First: how you verify anything

Get this wrong and every finding below is unreliable, because you will believe
an exit code that was never yours. It is the highest-frequency defect in this
repo's history — **four separate incidents in one day**, across three agents and
a human's own script.

- **`set -o pipefail`, in every shell you open.** Without it,
  `cargo build 2>&1 | tail` reports `tail`'s status. `cargo fmt --check` has
  reported green over a failing run here, and a verification script printed
  `verify exit 0` while `cargo build` was failing.
- **Never `cmd | tail && echo OK`.** The `&&` is testing the pipeline's _last_
  command.
- **`${PIPESTATUS[0]}` is the most recent pipeline's**, not the one you meant.
  If anything ran in between — even an `echo` in a pipe — it refers to that
  instead. Do not reach for it as a fix; use `pipefail`.
- **Read the output.** An exit code you did not print, from a command whose
  output you truncated, is not evidence. Say what you saw, not that it "passed".
- When a status matters, give the command its own invocation rather than
  chaining it behind `;` or `&&` with something that will overwrite `$?`.

The full local suite, which is what CI runs:

```sh
nix develop -c bash -c 'set -o pipefail
  cargo build --all && cargo test --all && cargo fmt --all -- --check \
    && cargo clippy --all-targets -- -D warnings'
nix develop -c bash -c 'set -o pipefail
  cd frontend && npm ci && npm run check && npm run lint && npm run test:unit \
    && npm run build && npm run build-storybook'
```

## Then: build the merged tree, before you read a line of the diff

**A PR's green CI says nothing once another PR has landed.** This is the
highest-yield check in this document and nothing in CI performs it.

It has now happened twice. The second time: #182 changed `runs::finish` to take
an `Outcome`; #177 added a `nutrition.rs` still passing `runs::COMPLETED`, and
#182 added a test constructing `Recipe` while #177 added two required fields to
it. **The files did not overlap at all.** Git merged clean, GitHub reported
`MERGEABLE` / `CLEAN`, and the last CI run on each branch was green because it
predated the other. Only building the merged tree caught it.

```sh
git fetch origin main
git merge --no-ff --no-commit origin/main   # merge main IN; never rebase a shared branch
nix develop -c bash -c 'set -o pipefail; cargo build --all && cargo test --all'
```

`MERGEABLE`/`CLEAN` from `gh pr view` means _git found no textual conflict_. It
is not a claim about types, about a function's arity, or about a struct's
required fields. Treat a green check older than the base it will merge into as
**no information**.

If the merge is dirty, resolving it is part of the change, not a separate errand
— merge `main` in, never rebase, and never `gh pr merge` your way around it.

## The checks

### 1. A fixture where an index is the fact

`buy_checks` and the pantry pre-ticks are keyed by **position in the shopping
list**, and that list is a _filtered_ projection (`shoppingLines` drops unread
lines). So an index is only meaningful against one exact ingredient list, and
nothing in the type system ties them together.

#157 replaced an 8-line fixture with the recipe's real 16 lines; #170's tick
maps were positions in the old list. The merge produced `salt` ticking
`cumin seeds` — a fixture asserting exactly the false pre-tick the matcher
exists to refuse — plus two stories pre-ticking an ingredient #157 had _deleted
for being invented_, and a "Nothing to buy" story ticking 8 of 16. Every check
was green. The only signal was the renders changing height.

**What to do:** for every `ticks:` map in a story, resolve the recipe's actual
line list (`corpus-sample.json` → the `structured`-and-non-blank filter that
`shoppingLines` applies) and check _both_ couplings: the integer is in range,
**and** the `inPantry("…")` name is the ingredient at that position. Do this
whenever either side moves — a fixture change and a tick-map change in the same
PR is the exact shape. Generalise: **any index into a list that lives somewhere
else.**

### 2. Invented fixture data

Story fixtures mirror **real** source records (CLAUDE.md) — an invented id or
image renders a recipe that does not exist. It has been violated more than once:
the Massaman `via` claimed "coconut milk", which is in neither adjacent recipe
and which the backend is tested never to produce.

`fixtures.ts` mostly enforces this itself: ids go through `row()`, which throws
if the id is not in `corpus-sample.json`. **Look for the fixtures that bypass
it** — a hand-typed card, a hand-typed `total_seconds`, an id constructed
inline. Each one is either a deliberate exception that says so in a comment, or
drift.

A hand-typed _value_ is as bad as a hand-typed id, and less visible: #84 shipped
an `UntimedRecipe` story using a recipe that has a time in production, with
invented round `total_seconds` that made an hours-formatting rule look correct
while a real 9000s rendered as "150 min+". Checking an id against
`corpus-sample.json` proves the fixture matches a **committed snapshot**, not
that it matches reality — for that, hand off to `audit-corpus`.

### 3. A derived column the sole writer never names

`recipes.equipment` sat at its `'[]'` default across **790 of 790 rows** for
months: migration 0014 added it, `derive` computed it, and `upsert` — the only
writer — never listed it. `fully_timed` (#171) was one review away from the same
fate.

**This is now a gate**, so do not re-derive it by hand:
`upsert_fills_every_column_the_schema_declares` and
`upsert_carries_every_column_on_update_too` in `backend/src/recipes.rs` read the
column list out of `PRAGMA table_info` and fail if any column comes back at its
default, on insert or on update.

Two things the tests still cannot prove, and you should:

- **The tests describe the writer, not the rows.** Fixing the writer does not
  fix rows written before the fix; they stay stale until a re-derive actually
  runs. If a change adds or repairs a derived column, the question "and what do
  the live rows say" is `audit-corpus`'s.
- **The same shape in tables the tests do not cover** — `raw_imports`,
  `ingredient_structures`, and the other capture tables have their own single
  writers.

### 4. A guard in a preceding read instead of the write's predicate

The TOCTOU shape: `SELECT` to check a condition, then `DELETE`/`UPDATE`/`INSERT`
that assumes it still holds. Two round trips, and the window between them is
real.

#169 got it right —
`EXISTS (SELECT 1 FROM pick_sessions WHERE … started_at IS NULL)` **inside** the
DELETE, so the check and the act are one statement. `set_buy_check` still does
the two-round-trip version (#175).

**What to do:** for every write in the diff, find the condition that makes it
legal and check whether that condition is in the write's own `WHERE`. If it is
in an earlier query, say so and name the interleaving that breaks it.

### 5. Migration numbering

`db.rs` applies by `MAX(version)`, so a lower number merged _after_ a higher one
has run on production **never applies at all** — silently, forever.

**The structural half is now a gate**: `migration_ledger_is_well_formed` in
`backend/src/db.rs` pins ascending order, that each entry embeds the file its
own number names, that every `.sql` on disk is registered, and that any hole is
declared in `RESERVED` with a reason.

**The half that needs production is yours to route.** No test can know the
highest version already applied — only `_migrations` in Turso knows. If the diff
adds a migration, its number must exceed that floor, and checking it is
`audit-corpus`'s `SELECT MAX(version) FROM _migrations`. A number that merely
looks like "the next one" on this branch is not enough: whichever branch deploys
first sets the floor, so a migration numbered below a _sibling branch's_
already-deployed one is dead on arrival. Filling a hole below the floor is the
same bug wearing a tidier hat.

### 6. A value nothing writes — and a value nothing distinguishes

Two directions of one defect, and a change that adds a variant usually needs
both.

**Nothing writes it.** `runs::FAILED` was reachable only from the CLI; every
service path hardcoded `COMPLETED`, so 223 production runs said completed and 0
said failed (#174). Generally: an enum variant, a constant, or a status that
**no reachable path constructs**. Rust's dead-code lint does not catch it — the
CLI constructs it, so it is "used".

**Nothing distinguishes it.** `HealthDashboard.badgeCls` matched `completed`,
`partial` and `failed`, then fell through to the in-flight amber for everything
else — so adding a new terminal status would have rendered it in the _in-flight_
colour, which is worse than rendering it in no colour, because it is confidently
wrong. Generally: **a value the code now accepts that no reader distinguishes.**

**What to do:** when a diff adds a variant, enumerate every producer (does
anything construct it?) and every consumer (does anything branch on it, and what
does the fallthrough do?). A `default:`/`else` that quietly absorbs a new value
is a finding, not a style note.

### 7. The `$env/dynamic/public` value-import trap

Importing a **value** from a module that reads `$env/dynamic/public` drags that
read into the Storybook bundle, where it is `undefined` and crashes every story
of the component — while check, lint and build all stay green, and the visual
fence photographs the crash quite happily.

**This is now a gate**: `lint:env` (`frontend/scripts/lint-env.mjs`) walks the
value- import graph from every `*.stories.ts` and `*.test.ts` and fails with the
chain. What is left for you is the _design_ question the lint cannot ask: when a
diff adds a module, is the pure/impure split still in the right place, or did a
helper that should have gone in `$lib/shopping` land in `$lib/buy` and get
imported as a type today, as a value tomorrow?

### 8. A claim nobody measured

"92 recipes claim under 10 minutes", "only 1 kitchen, holding 0 equipment",
"2,072 of 9,152 steps are timed" — a PR body, a code comment, or an issue
argument that states a fact about production. **Every one of these is either
measured or a guess**, and a guess dressed as a measurement is the thing to
catch.

If the diff or its PR body asserts a number about the corpus, it needs a query
behind it. Hand it to `audit-corpus`; do not accept "roughly" and do not invent
one yourself.

### 9. Blessing a baseline nobody read

`visual:update` is the one way to defeat the fence, so re-blessing is a
conscious act.

The trap: **a size change emits no triptych**. The one case where the render
changed most is the case with the least to look at. So on any PR that touches
committed baselines:

- Confirm the captured shot count equals `index.json`'s entry count — a story
  that failed to render is a story with no diff.
- **Read every triptych** in `tests/visual/__diff__/` (baseline | current |
  diff).
- For a story whose baseline changed size, there is no triptych — **read the
  `current/` render directly.**
- Then ask the question the fence cannot: _is this what the PR says it did?_ A
  baseline updated in the same commit as an unrelated refactor is the shape to
  distrust.

### 10. The authoritative value fetched and then discarded

`serverParticipants` is assigned from every tally and commented "authoritative
count from the last tally" — and read nowhere, while consensus is computed from
a locally derived count instead (#181). The comment is the tell: it _claims_
authority for a value that has no effect.

Nothing warns about this. It is an assignment, so it is not an unused variable;
it is `$state`, so Svelte is content. Generally: **the authoritative value is
fetched and then discarded**, and a second, weaker derivation is used in its
place.

**What to do:** for each value the diff fetches from the server or computes as
canonical, grep for its reads. A write-only value is either dead (delete it) or
the symptom that the wrong value is being used downstream (fix that). Comments
claiming authority — "authoritative", "source of truth", "canonical" — are the
places to look first.

## Reporting

For each finding, three things and no filler:

1. **The claim the code makes** — what a reader would reasonably believe from
   it.
2. **The fact that contradicts it** — the line, the row count, the missing
   branch.
3. **The fix**, concretely.

Rate by **production impact**, not by how clever the finding is: a fixture
asserting a false match misleads every future reviewer; a write-only variable
may be cosmetic.

Say plainly which checks you ran and which you could not (no credential, no
render, no merged build). **Do not report "looks correct"** — the absence of a
finding is not a finding, and this repo's whole premise is that everything
already looks correct.

## Do / don't

- **Do** build the merged tree before reading the diff.
- **Do** `set -o pipefail` in every shell, and read output rather than trusting
  a status you did not print.
- **Do** read every triptych, and the `current/` render for a size change.
- **Do** hand corpus questions to `audit-corpus` rather than guessing a number.
- **Don't** re-check what `lint:design`, `lint:stories`, `lint:env`, the visual
  fence, `svelte-check`, `clippy`, or the migration/column tests already prove.
- **Don't** treat `MERGEABLE`/`CLEAN`, or a green check older than the base, as
  evidence.
- **Don't** run `visual:update` to make a build green. Read the render and
  decide.
- **Don't** go looking for credentials, and never use the Turso **write** token.
