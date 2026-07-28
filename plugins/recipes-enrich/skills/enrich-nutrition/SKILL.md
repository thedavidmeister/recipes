---
name: enrich-nutrition
description: >-
  Run the recipes nutrition-reading worker. Pull recipes whose calories have not been
  read, read each one into a per-ingredient energy density (and, where asked, the weight
  of one unit) plus how many people it serves, and push the results back — a fixed
  pull → read → push loop until the queue is empty. Use when reading the recipes
  corpus's nutrition (a cron, or "read the calories"). The two tools do all I/O; you do
  only the reading, and never any arithmetic.
---

# Read what each recipe costs you

You are the nutrition-reading worker. Your whole job is a loop:

1. **pull** the recipes whose nutrition still needs reading (the
   `nutrition_pull` tool),
2. **read** each one into per-ingredient food facts and a serving count (this is
   the model work),
3. **push** the readings back (the `nutrition_push` tool),
4. repeat until nothing is pending.

You have exactly two tools: **`nutrition_pull`** and **`nutrition_push`** (from
the `recipes-enrich` plugin's MCP server). They talk to the app for you — the
**app** does the validation, storage, and bookkeeping. You never touch the
database, never run `git`, never read the repo.

Keep it tight: no prose, no explanation, no exploring. Call the two tools, do
the reading between them, stop when the queue is empty.

## The one rule that matters most: you do not add up

A recipe's calories are arithmetic — quantity × weight × density, summed. **The
app does all of it.** You are never asked for a recipe's total, a per-serving
figure, or a line's calories.

You are asked only for facts about **foods**, which do not change from one
recipe to the next:

- how many kcal are in 100 g of this food, and
- how much one of this line's unit weighs, where the app cannot work that out.

If you find yourself multiplying, you have misread the task. A total you produce
is a plausible-looking number nobody can check; a density you produce is a fact
that is either right or wrong by an amount someone could look up.

## The loop

### 1. Pull

Call **`nutrition_pull`** (optionally with `limit`, e.g. 25). It returns recipes
whose nutrition has no reading yet:

```json
[
  {
    "source": "themealdb",
    "id": "52772",
    "title": "Teriyaki Chicken Casserole",
    "instructions": "Preheat oven to 350°F… pour into a 9x13 dish… serve over rice.",
    "ingredients": [
      {
        "name": "chicken",
        "measure": "3/4 pound",
        "item": "chicken breasts",
        "unit": "lb",
        "weighable": true
      },
      {
        "name": "soy sauce",
        "measure": "1/2 cup",
        "item": "soy sauce",
        "unit": "cup",
        "weighable": false
      },
      {
        "name": "eggs",
        "measure": "2",
        "item": "eggs",
        "unit": null,
        "weighable": false
      }
    ]
  }
]
```

**If the array is empty, STOP — the queue is drained. You are done.**

Only recipes whose ingredient measures have already been read are offered, so
every line has a quantity the app can multiply.

### 2. Read the nutrition

Produce, for each recipe, one entry per ingredient **in the order given**, plus
a serving count:

```
FoodEnergy = {
  "kcal_per_100g": number,          // always
  "grams_per_unit": number          // only when "weighable" is false
}
```

#### `kcal_per_100g` — the food's energy density

Per **100 g**, which is how every nutrition label and food database states it,
so you are recalling a figure rather than converting one.

- Read the food **as it goes into the pot**: raw chicken breast, not roasted;
  dried pasta, not cooked.
- `0` is a real answer. Water, salt, black pepper, most spices in the amounts
  used, and vinegar are all effectively zero. Say `0`, do not omit the line.
- Nothing edible is above **900** (pure fat). If you are about to write a number
  in the thousands you have produced a recipe total or a per-pound figure by
  mistake — the app will refuse it.

Rough anchors, so the readings across a run stay consistent with each other:
water/stock 0–10 · most vegetables 15–50 · fruit 30–70 · cooked rice/pasta
110–160 · dried rice/pasta 350–370 · lean meat and fish 100–200 · fatty meat
250–400 · bread 250–280 · flour and sugar 360–400 · cheese 300–400 · nuts
550–650 · butter 720 · oil 880–900.

#### `grams_per_unit` — how much one of _this line's_ unit weighs

**Only where `weighable` is `false`.** When it is `true` the app already knows
(the unit is a mass unit, or the line states a size like `1 (14 oz) can`) and
anything you send is ignored — it will not let a reading disagree with its
conversion table.

The unit is the one in the `unit` field, and `null` means a bare count, which
still has a unit: one egg, one onion, one lemon.

- `"unit": "cup"` → the weight of **one cup** of this food. A cup of flour is
  ~125 g, of sugar ~200 g, of water ~236 g, of chopped onion ~160 g.
- `"unit": "clove"` → ~3 g. `"unit": "tbsp"` of oil → ~14 g.
- `"unit": null`, `"item": "eggs"` → ~50 g (one egg).
- `"unit": "can"` with no stated size → the drained-or-not weight you would
  expect of that can, e.g. ~400 g.

If you genuinely cannot say — a unit that means nothing on its own, like
`"splash"` or `"handful"` on an ingredient you cannot picture — **omit
`grams_per_unit`**. The app counts that line as a gap and marks the recipe's
total as a floor, which is honest. Do not invent a number to avoid the flag.

#### `servings` — how many people it feeds

One number for the whole recipe, and it is **required**.

No source in the corpus states a yield, so this is a reading like everything
else. That is not a reason to duck it: a bare calorie total is useless — 2,400
kcal is a reasonable tray of lasagne and an absurd plate of it — and every
recipe feeds a definite number of people whether or not anyone wrote it down.

Read it the way a cook would, from the whole recipe at once:

- **The quantities.** 500 g of pasta serves 4–5. 4 chicken breasts serves 4. One
  egg serves 1.
- **The vessel.** A 9x13 dish is 6–8. A 20 cm cake tin is 8–10. A loaf is 10–12
  slices.
- **The wording.** "divide between four bowls", "serve over rice for two".
- **The kind of dish.** A sauce, a dressing or a spice mix serves as many as the
  thing it dresses — read it as the number of portions of the finished dish.

Give a single whole number, not a range: pick the middle of what you would say.
It must be between 1 and 100.

### 3. Push

Call **`nutrition_push`** with one entry per recipe:

```json
[
  {
    "source": "themealdb",
    "id": "52772",
    "servings": 6,
    "foods": [
      { "kcal_per_100g": 165 },
      { "kcal_per_100g": 53, "grams_per_unit": 255 },
      { "kcal_per_100g": 143, "grams_per_unit": 50 }
    ]
  }
]
```

One entry per ingredient, **in the pulled order**, no gaps — the app matches
them by position and refuses a batch whose count has drifted.

The app returns `{ accepted, derived, rejected }`. **Read the rejections** —
each carries a reason:

- _"reading count … does not match"_ — you skipped or added a line, or the
  recipe changed under you. Re-read that recipe on the next pull.
- _"nothing edible exceeds 900"_ — a density is a total or a per-pound figure.
- _"servings is …"_ — outside 1–100.
- _"one of something weighs more than nothing"_ — a zero, negative or absurd
  `grams_per_unit`.
- _"no such recipe"_ — it disappeared from the corpus; skip it.

A rejected recipe stays in the queue and will come back on the next pull.

### 4. Repeat

Pull again. Stop when the array comes back empty.
