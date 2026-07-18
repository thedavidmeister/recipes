---
name: enrich
description: >-
  Run the recipes ingredient-enrichment worker. Pull recipes that still need their
  raw ingredient lines read into structured measures, read each line, and push the
  results back — a fixed pull → extract → push loop until the queue is empty. Use
  when enriching the recipes corpus (a cron, or "run enrichment"). The binary does
  all I/O; you do only the reading.
---

# Enrich the recipes corpus

You are the enrichment worker. Your whole job is a loop:

1. **pull** the recipes that still need reading (a binary command),
2. **read** each raw ingredient line into a structured measure (this is the only
   part that is yours — the model work),
3. **push** the readings back (a binary command),
4. repeat until nothing is pending.

The binary `recipe-backend` is your only tool. It does every bit of I/O —
talking to the database, the run bookkeeping, validation, storage. You never
touch the database, never run `git`, never read the repo. You read lines and
emit JSON.

Keep it tight: no prose, no explanation, no exploring. Run the two commands, do
the reading between them, stop when the queue is empty.

## The loop

### 1. Pull

```
recipe-backend enrich pull --limit 25
```

Prints a JSON array of recipes that have no reading yet:

```json
[
  {
    "source": "themealdb",
    "id": "52772",
    "ingredients": [
      { "name": "Chicken", "measure": "1 whole" },
      { "name": "Salt", "measure": "to taste" }
    ]
  }
]
```

**If the array is empty, STOP — the queue is drained. You are done.**

### 2. Read each line into a `StructuredMeasure`

For every recipe, produce **one reading per ingredient line, in the same
order**. The reading count for a recipe MUST equal its number of ingredient
lines, or the push rejects it.

The shape (this is the contract — match it exactly):

```
StructuredMeasure = {
  "item":        string,          // the ingredient itself, taken from the line
  "amount":      null | Amount,   // null when the line states no quantity at all
  "preparation": null | string,   // "minced", "finely chopped"
  "note":        null | string    // "to serve", "optional", "plus extra"
}

Amount =
  | { "kind": "quantified", "quantity": Quantity, "unit": null | string, "size": null | Size }
  | { "kind": "qualitative", "text": string }        // "to taste", "a pinch", "a splash"

Quantity =
  | { "kind": "exact", "value": number }
  | { "kind": "range", "low": number, "high": number }   // "2-3"

Size = { "quantity": Quantity, "unit": null | string }    // "1 (14 oz) can" → size {14, "oz"}
```

Reading rules:

- **`item`** is the ingredient, taken from the line — **never invent** an
  ingredient the line does not name. If `name` is the ingredient, use it. If the
  item is folded into the measure text (name empty, measure `"1 cup flour"`),
  pull it out of there.
- **`amount`** is `null` when the line states no quantity at all.
- A plain number is `{ "kind": "exact" }`. A range like `"2-3"` is
  `{ "kind": "range" }`.
- A phrase with no number — `"to taste"`, `"a pinch"`, `"a splash"` — is a
  **qualitative** amount. Do **not** invent a number for it.
- Put preparation (`"minced"`, `"finely chopped"`) in `preparation`, and
  anything else the line carries (`"to serve"`, `"optional"`, `"plus extra"`) in
  `note`. Leave each `null` when absent.
- A size annotation like `"1 (14 oz) can"` is quantity `1`, unit `"can"`, size
  `{ quantity 14, unit "oz" }`.
- **Do not convert units or do any arithmetic** — record what the line says.
  Conversion and scaling happen deterministically downstream. This is the one
  rule it is easiest to break: `"1 cup"` stays `1` + `"cup"`, never `240` +
  `"ml"`.

### 3. Push

Write the readings as a JSON array to a temp file, then feed it to the binary.
Each entry is a recipe key plus its readings (no model field — the binary stamps
that):

```json
[
  {
    "source": "themealdb",
    "id": "52772",
    "readings": [
      {
        "item": "chicken",
        "amount": {
          "kind": "quantified",
          "quantity": { "kind": "exact", "value": 1 },
          "unit": "whole",
          "size": null
        },
        "preparation": null,
        "note": null
      },
      {
        "item": "salt",
        "amount": { "kind": "qualitative", "text": "to taste" },
        "preparation": null,
        "note": null
      }
    ]
  }
]
```

```
recipe-backend enrich push < /tmp/enrich-batch.json
```

It prints what happened:

```json
{ "accepted": 1, "derived": 1, "rejected": [] }
```

- `accepted` — readings stored.
- `derived` — recipes rebuilt so the readings show immediately.
- `rejected` — submissions dropped, each with a reason (usually the reading
  count no longer matched the recipe because its raw changed since the pull). A
  rejected recipe simply comes back in the next pull.

### 4. Loop

Go back to step 1. Stop when:

- **pull returns an empty array** (the queue is drained — the normal finish), or
- **a push reports `accepted: 0` for a non-empty batch** (every recipe was
  rejected — something is wrong; stop and report the reasons rather than
  spinning).

## Do / don't

- **Do** use only `recipe-backend enrich pull` and `recipe-backend enrich push`.
- **Do** produce exactly one reading per ingredient line, in order.
- **Do** keep your output to the JSON — no commentary, no markdown fences in the
  file.
- **Don't** do arithmetic or unit conversion, ever.
- **Don't** invent ingredients, quantities, or notes the line does not contain.
- **Don't** read the repo, edit files, or run any other command.

## Setup (the cron provides this)

`recipe-backend` reads the corpus directly, so its environment must carry:

- `DATABASE_URL` + `TURSO_AUTH_TOKEN` — the corpus to pull from and push to.
- `ENRICH_MODEL` — recorded as each reading's provenance (e.g.
  `claude-opus-4-8`).

If `recipe-backend enrich pull` errors with a database or auth message, that env
is missing — stop and say so; do not try to work around it.
