---
name: enrich-equipment
description: >-
  Run the recipes equipment-reading worker. Pull recipes whose required equipment has
  not been read, read each method into the list of things you must own to cook it —
  preparation tools as well as appliances — and push the results back, a fixed
  pull → read → push loop until the queue is empty. Use when reading the recipes
  corpus's equipment (a cron, or "read the equipment"). The two tools do all I/O; you
  do only the reading.
---

# Read what each recipe needs you to own

You are the equipment-reading worker. Your whole job is a loop:

1. **pull** the recipes whose equipment still needs reading (the
   `equipment_pull` tool),
2. **read** each method into the list of equipment it requires (this is the
   model work),
3. **push** the readings back (the `equipment_push` tool),
4. repeat until nothing is pending.

You have exactly two tools: **`equipment_pull`** and **`equipment_push`** (from
the `recipes-enrich` plugin's MCP server). They talk to the app for you — the
**app** does the validation, storage, and bookkeeping. You never touch the
database, never run `git`, never read the repo. You read methods and call the
two tools.

Keep it tight: no prose, no explanation, no exploring. Call the two tools, do
the reading between them, stop when the queue is empty.

## Why this reading is different

What you produce is a **vocabulary**, not just an annotation. A kitchen tells
the app what it owns by picking from the list of everything the corpus has ever
asked for — it cannot type its own. So two spellings of one thing are not
untidy, they are two different items that can never match each other, and a
kitchen that owns "frying pan" gets no credit for a recipe that asked for
"Frying Pan".

That is why the app **refuses** a reading rather than tidying it.

## The loop

### 1. Pull

Call **`equipment_pull`** (optionally with `limit`, e.g. 25). It returns recipes
whose equipment has no reading yet:

```json
[
  {
    "source": "themealdb",
    "id": "52772",
    "instructions": "Preheat the oven to 180C. Chop the onions finely on a board. Fry in a large pan, then bake for 40 minutes."
  }
]
```

**If the array is empty, STOP — the queue is drained. You are done.**

### 2. Read the equipment

Produce, for each recipe, the things a person must **own** to cook it:

```
RequiredEquipment = { "item": string }   // normalised name
```

**Read for preparation as well as cooking.** This is the mistake to avoid: a
salad needs a bowl, a knife and a board even though nothing is heated. A reading
that lists only the obvious machinery is wrong, and it is wrong in a way that
matters — a kitchen owning no knife would appear able to cook everything.

Include: pans, pots, trays, boards, bowls, knives, whisks, spoons, colanders,
graters, ovens, hobs, blenders, mixers, and anything else the method genuinely
requires.

Exclude:

- **Ingredients.** They are read elsewhere and are not owned.
- **Things a kitchen is assumed to have** only if the method never touches them
  — do not invent an oven for a no-cook dish.
- **Brands, sizes and materials** unless the method truly depends on them.
  "pan", not "12-inch nonstick pan". Prefer the coarser name a person would use
  when saying what they own.

### Names must arrive normalised

- **lowercase** — `wok`, never `Wok`
- **trimmed**, with single spaces — `chopping board`, never `chopping  board`
- **no duplicates** within one recipe

The app refuses anything else rather than repairing it, so a rejected batch
means the names were wrong, not that the app was fussy.

Reuse a name you have already used in this run when it is genuinely the same
thing. The vocabulary is only as useful as it is consistent.

### 3. Push

Call **`equipment_push`** with one entry per recipe:

```json
[
  {
    "source": "themealdb",
    "id": "52772",
    "equipment": [
      { "item": "chopping board" },
      { "item": "knife" },
      { "item": "frying pan" },
      { "item": "oven" },
      { "item": "baking tray" }
    ]
  }
]
```

The app returns `{ accepted, derived, rejected }`. **Read the rejections** —
each carries a reason:

- _"empty equipment reading"_ — you listed nothing. Every recipe needs
  something.
- _"not normalised"_ — fix the spelling and resubmit that recipe.
- _"repeats"_ — the same item twice in one recipe.
- _"no such recipe"_ — it disappeared from the corpus; skip it.

A rejected recipe stays in the queue and will come back on the next pull.

### 4. Repeat

Pull again. Stop when the array comes back empty.
