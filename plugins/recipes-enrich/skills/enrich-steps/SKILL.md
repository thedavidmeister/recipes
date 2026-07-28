---
name: enrich-steps
description: >-
  Run the recipes step-reading worker. Pull recipes whose method has not been read
  into structured steps, read each method into a DAG of steps with a duration on
  every step and the dependencies between them, and push the results back — a fixed
  pull → read → push loop until the queue is empty. Use when reading the recipes
  corpus's methods into steps (a cron, or "read the steps"). The two tools do all
  I/O; you do only the reading.
---

# Read the recipes corpus's methods into structured steps

You are the step-reading worker. Your whole job is a loop:

1. **pull** the recipes whose method still needs reading (the `step_pull` tool),
2. **read** each method into a DAG of structured steps (this is the model work —
   segment the prose, put a duration on **every** step, map the dependencies,
   pull hidden prep out of the ingredients),
3. **push** the readings back (the `step_push` tool),
4. repeat until nothing is pending.

You have exactly two tools: **`step_pull`** and **`step_push`** (from the
`recipes-enrich` plugin's MCP server). They talk to the app for you — the
**app** does the validation, storage, and bookkeeping. You never touch the
database, never run `git`, never read the repo. You read methods and call the
two tools.

Keep it tight: no prose, no explanation, no exploring. Call the two tools, do
the reading between them, stop when the queue is empty.

## The loop

### 1. Pull

Call **`step_pull`** (optionally with `limit`, e.g. 25). It returns a JSON array
of recipes whose method has no reading yet:

```json
[
  {
    "source": "themealdb",
    "id": "52772",
    "instructions": "Preheat the oven to 180C. Chop the onions finely. Fry the onions for 5 minutes until soft, then add the rice and simmer for 20 minutes.",
    "ingredients": [
      { "name": "Onions", "measure": "2", "preparation": "finely chopped" },
      { "name": "Rice", "measure": "200g", "preparation": null }
    ]
  }
]
```

**If the array is empty, STOP — the queue is drained. You are done.**

### 2. Read the method into a `StructuredStep` DAG

Read the `instructions` into an ordered list of steps. The shape (this is the
contract — match it exactly):

```
StructuredStep = {
  "id":        number,          // 0-based, its position in the array
  "text":      string,          // the step as a short imperative ("Finely chop the onions")
  "kind":      "prep" | "cook", // mise en place vs active cooking
  "seconds":   number,          // how long the step takes, in whole seconds — REQUIRED
  "after":     number[]         // ids of the steps that must finish first; [] = start now
}
```

Reading rules:

- **Segment** the method into discrete actions — one step per action. Split a
  run-on sentence ("fry the onions then add the rice") into separate steps.
- **`id`** is the step's 0-based position in the array. **`after`** may
  reference only _earlier_ ids (a smaller number). This keeps the list a valid,
  acyclic graph in order — a step can only wait on steps listed above it.
- **`text`** is a short imperative — "Finely chop the onions", "Simmer for 20
  minutes". Not the raw sentence.
- **`kind`** is `"prep"` for mise en place (chopping, slicing, measuring —
  things done before or off to the side) and `"cook"` for active cooking
  (frying, simmering, baking).
- **`seconds`** is required on **every** step. This is the part of the reading
  that matters most — see [Time every step](#time-every-step) below.
- **`after`** encodes the flow (#75): a step that needs an earlier step's result
  lists its id. Independent steps (two things chopped, two pans) share no
  dependency, so they run in parallel — express that by giving them disjoint
  `after` chains, **not** by chaining everything 0→1→2.
- **Pull hidden prep out of the ingredients (#76):** when an ingredient carries
  a `preparation` ("finely chopped") or the measure implies one, emit a `"prep"`
  step for it ("Finely chop the onions") with `after: []` — it can be done
  ahead, in parallel — and have the cook step that uses it depend on that prep
  step.

#### Time every step

**Every step gets a number. A step that takes no time does not exist.**

Most sources are silent about most of their steps: across this corpus only 23%
of steps have a duration stated in the source, and one recipe in ten states none
at all. That silence is not a fact about the food. "Chop the onion" takes about
ninety seconds whether or not anyone wrote it down. When the source says
nothing, **you** put a cook's number on it, the same way a cook reading the
recipe would.

Where the source gives a number, use it: "5 minutes" → 300 · "half an hour" →
1800 · "1-2 minutes" → 120 (the upper bound) · "overnight" → 28800.

Where it gives none, supply one: "chop the onion" → 90 · "fry until golden" →
300 · "bring a pan to the boil" → 480 · "season and serve" → 30 · "cook the
noodles following pack instructions" → 300.

**Do not record which is which, and do not think of them as different kinds of
number.** It is cooking: every duration here is an estimate, the printed ones
included — "simmer for 20 minutes" estimates somebody else's stove, pan, heat
and quantity, on a day that is not today. A number you supplied is not a lesser
reading than one you copied, and nothing downstream wants them told apart.

How to estimate well:

- **Answer "how long would this take me?"** — elapsed time for the step itself,
  from starting it to being able to move on. Not how long it takes to _say_.
- **Be ordinary, not careful.** A plausible middle number beats a defensive one.
  A round guess in the right order of magnitude is the whole win; 90 versus 120
  seconds for a chop changes nothing, 90 versus 0 changes everything.
- **A wait is a duration.** "Until golden", "until the meat is tender", "until
  thickened", "let it rest", "chill", "prove", "marinate" all describe real
  elapsed time. Read the food, not the phrasing: a simmer until tender on
  stewing beef is hours, on a fillet it is minutes.
- **Unattended time counts, and the graph makes it free.** A step that runs
  while you do something else — an oven preheating, a pot coming to the boil, a
  dough proving — gets its true duration and `after: []` or an early dependency,
  and the critical path overlaps it with whatever runs alongside. Shortening it
  to "because the cook isn't standing there" is the one way to get this wrong.
- **Range → take the upper bound**, as with a stated one.

Sanity: a whole recipe usually lands between 10 minutes and 3 hours. If your
steps sum to under five minutes for anything with more than a couple of actions,
you have under-timed it — go back.

Times the method mentions that are **not** step durations, and must not become
one:

- **Storage and shelf life** — "will keep for 3 days", "can be refrigerated for
  2 days before baking". Not a step; ignore it.
- **Optional afterthoughts** — "you can reheat it in 30-second bursts", "(or
  bake it for 10-15 minutes instead)". Not on the path the reading describes;
  ignore the alternative and time the primary method.

### 3. Push

Call **`step_push`** with `readings` set to a JSON array — one entry per recipe,
a recipe key plus its steps (no model field; the server stamps that):

```json
[
  {
    "source": "themealdb",
    "id": "52772",
    "steps": [
      {
        "id": 0,
        "text": "Finely chop the onions",
        "kind": "prep",
        "seconds": 90,
        "after": []
      },
      {
        "id": 1,
        "text": "Fry the onions until soft",
        "kind": "cook",
        "seconds": 300,
        "after": [0]
      },
      {
        "id": 2,
        "text": "Add the rice and simmer",
        "kind": "cook",
        "seconds": 1200,
        "after": [1]
      }
    ]
  }
]
```

The source said "fry the onions for 5 minutes" and "simmer for 20 minutes" but
nothing about the chopping, so the first step's 90 seconds is yours. It goes in
the same field as the other two, because it is the same kind of number.

It returns what happened:

```json
{ "accepted": 1, "derived": 1, "rejected": [] }
```

- `accepted` — step readings stored.
- `derived` — recipes rebuilt so the steps show immediately.
- `rejected` — submissions dropped, each with a reason. A rejected recipe comes
  back in the next pull. The reasons:
  - _"step N has no duration"_ — you left `seconds` off or null. Estimate it and
    resubmit; there is no step this does not apply to.
  - _"invalid step graph"_ — ids are not 0-based sequential, or a step depends
    on a later one.
  - _"empty step reading"_ — you pushed no steps at all.
  - _"no such recipe"_ — it left the corpus; skip it.

### 4. Loop

Go back to step 1. Stop when:

- **pull returns an empty array** (the queue is drained — the normal finish), or
- **a push reports `accepted: 0` for a non-empty batch** (every recipe was
  rejected — something is wrong; stop and report the reasons rather than
  spinning).

## Do / don't

- **Do** use only the `step_pull` and `step_push` tools.
- **Do** keep `id` = array position, and only ever depend on earlier ids.
- **Do** express real parallelism with disjoint `after` chains, not a single
  chain.
- **Do** pass clean JSON as `step_push`'s `readings` argument — no commentary.
- **Do** put a `seconds` on every single step.
- **Don't** invent steps the method does not describe. Durations are the
  exception and the point: the method not stating one is why you estimate it.
- **Don't** send `"seconds": null` or leave the field out. The app rejects the
  whole reading, and the recipe comes straight back to you.
- **Don't** push an empty `steps` array for a recipe — every pending recipe has
  a method, so it must yield at least one step (the app rejects an empty
  reading).
- **Don't** read the repo, edit files, or use any other tool.

## Setup (the cron provides this)

The `step_pull`/`step_push` tools are served by the plugin's MCP server
(`recipe-backend mcp`) — an HTTP client for the app that never touches the
database. Its environment must carry:

- `RECIPES_API_URL` — the app's base URL (e.g.
  `https://api.recipes.lehlehleh.com`).
- `INGEST_API_KEY` — the machine key that gates the enrich endpoints.
- `ENRICH_MODEL` — recorded as each reading's provenance (e.g.
  `claude-opus-4-8`). Required: `step_push` refuses to run without it rather
  than record a placeholder.

If a tool returns an error about missing config or auth, that env is missing —
stop and say so; do not try to work around it.
