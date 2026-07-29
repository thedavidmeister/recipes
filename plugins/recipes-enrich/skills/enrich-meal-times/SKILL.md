---
name: enrich-meal-times
description: >-
  Run the recipes meal-time worker. Pull recipes whose sittings have not been read, read
  each dish into the set of sittings it suits — breakfast, lunch, dinner, snack — and push
  the results back, a fixed pull → read → push loop until the queue is empty. Use when
  reading the recipes corpus's meal times (a cron, or "read when each dish is eaten").
  The two tools do all I/O; you do only the reading.
---

# Read when each dish is eaten

You are the meal-time worker. Your whole job is a loop:

1. **pull** the recipes whose sittings still need reading (the `meal_times_pull`
   tool),
2. **read** each dish into the set of sittings it suits (this is the model work),
3. **push** the readings back (the `meal_times_push` tool),
4. repeat until nothing is pending.

You have exactly two tools: **`meal_times_pull`** and **`meal_times_push`** (from
the `recipes-enrich` plugin's MCP server). They talk to the app for you — the
**app** does the validation, storage, and bookkeeping. You never touch the
database, never run `git`, never read the repo.

Keep it tight: no prose, no explanation, no exploring. Call the two tools, do
the reading between them, stop when the queue is empty.

## The one rule that matters most: it is a set, and it is never empty

A dish is not "a lunch". A chicken curry is **lunch or dinner**. A sandwich is
**lunch or a snack**. A roast is **dinner**. So the answer is the *set* of
sittings the dish suits — every one it genuinely suits, not the likeliest one.

Getting this wrong in either direction breaks the thing this reading is for:

- **Too few** and a real dinner is kept out of a dinner plan. Naming one sitting
  when the dish suits two is the common failure.
- **Too many** and the reading says nothing. All four is almost always a refusal
  to read rather than a fact about the food.

And **never empty**. Every dish is eaten at some time; an empty set would be your
reading failing, not a discovery about the food. The app refuses one, and the
recipe comes back on the next pull.

## The loop

### 1. Pull

Call **`meal_times_pull`** (optionally with `limit`, e.g. 25). It returns recipes
whose sittings have no reading yet:

```json
[
  {
    "source": "themealdb",
    "id": "52940",
    "title": "Brown Stew Chicken",
    "category": "Chicken",
    "area": "Jamaican",
    "instructions": "Squeeze lime over chicken… brown in oil… simmer 45 minutes… serve with rice and peas.",
    "ingredients": ["chicken", "lime", "onion", "garlic", "rice"]
  }
]
```

**If the array is empty, STOP — the queue is drained. You are done.**

### 2. Read the sittings

Produce, for each recipe, the set of sittings it suits, from exactly these four
words:

```
"breakfast" | "lunch" | "dinner" | "snack"
```

Anything else — `brunch`, `supper`, `Dinner`, `elevenses` — is refused at the
wire. Lowercase, always.

Read the **dish**, from everything the pull gives you at once:

- **The title and ingredients.** Oats, eggs, bacon and pancakes read breakfast.
  A joint of meat, a whole fish, a braise reads dinner.
- **The effort and the method.** "Simmer 3 hours" is not a breakfast; "toast the
  bread" is not a dinner on its own. A 10-minute assembly reads lunch or snack.
- **The size.** A single-portion bite is a snack. A tray, a roasting tin or a pot
  for the table is dinner.
- **The cuisine (`area`), which matters more than people expect.** Congee, a full
  English, shakshuka and pho are all breakfasts in their own kitchens. Read the
  dish where it comes from, not where you are.
- **The category.** It is a hint, not the answer — most of the corpus's
  categories (`Beef`, `Pasta`, `Vegetarian`, `Miscellaneous`) say what is in the
  dish and nothing about when it is eaten. But where the source *did* state
  something, do not contradict it without a reason: a `Breakfast` category is a
  breakfast, and it may well be a snack too.

Anchors, so readings stay consistent across a run:

| the dish | sittings |
|---|---|
| pancakes, porridge, a fry-up, shakshuka | `["breakfast"]` |
| toast, a muffin, banana bread, a smoothie | `["breakfast","snack"]` |
| a sandwich, a wrap, a filled roll | `["lunch","snack"]` |
| a salad, a soup, an omelette, a quiche | `["lunch","dinner"]` |
| a curry, a stir fry, a pasta, a stew, a bake | `["lunch","dinner"]` |
| a roast, a joint, a whole fish, a 3-hour braise | `["dinner"]` |
| crisps, dips, biscuits, a slice of cake, a truffle | `["snack"]` |
| a dessert or a pudding | the sitting it *follows* — usually `["dinner"]`, and `["lunch","dinner"]` if it would as readily follow a lunch |

**Accompaniments are read too.** Desserts, sides and starters come through this
queue like everything else, and they get the sittings they are eaten at. A meal
round already excludes them separately (they accompany a meal rather than being
one) — that is not your call to make here, and giving a trifle an empty set is
still a failed reading.

Two sittings is the most common right answer. One is common. Three is unusual and
wants a reason (a boiled egg, a flatbread). Four means you did not read the dish.

### 3. Push

Call **`meal_times_push`** with one entry per recipe:

```json
[
  { "source": "themealdb", "id": "52940", "sittings": ["lunch", "dinner"] },
  { "source": "themealdb", "id": "52855", "sittings": ["snack"] }
]
```

Order within `sittings` does not matter — the app stores the set canonically —
but each word may appear only once.

The app returns `{ accepted, derived, rejected }`. **Read the rejections** —
each carries a reason:

- _"every dish is eaten at some time"_ — you sent an empty set. Read the dish
  again; there is always an answer.
- _"repeats"_ — the same sitting twice in one set.
- _"no such recipe"_ — it disappeared from the corpus; skip it.
- _"superseded"_ — a newer run already read it; skip it.

A rejected recipe stays in the queue and will come back on the next pull.

### 4. Repeat

Pull again. Stop when the array comes back empty.
