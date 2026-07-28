<script lang="ts">
  import Alert from "./Alert.svelte";
  import Button from "./Button.svelte";
  import Notice from "./Notice.svelte";
  import UserName from "./UserName.svelte";
  import type { Tick } from "$lib/buy";
  import type { BuyRecipe, BuyStatus, StructuredMeasure } from "$lib/types";
  import { userAccent, userTint } from "$lib/colour";
  import { formatAmount } from "$lib/measure";

  /**
   * `buy` (#36): the shopping **checklist** for the pick's consensus recipe.
   *
   * The step after `pick` — what the group agreed on, and what it needs. Each line
   * ticks off as you shop, and a ticked line is *somebody's*: it wears the colour
   * of whoever got it and says their name (#131/#145), so a group shopping apart
   * can see at a glance what is already got and whose. Every state is a story.
   *
   * Each line is the structured reading (#11): the `item` to get, and how much —
   * the measured `amount`, or the `note` when a line states no quantity ("for
   * frying"). Never the raw measure; preparation belongs to `cook`.
   *
   * The colour is never the only signal. A ticked line is a checked box, a
   * struck-through name and a bumped counter first; the tint and the checkbox's
   * accent say whose on top of that. That is what lets every one of the six slots
   * be used unconditionally, including the two too pale to be a boundary on their
   * own (see `$lib/colour`).
   *
   * A line can also be ticked with **nobody behind it** (#156). The plan's kitchen
   * carries a pantry, and what it already holds starts ticked — so the salt is
   * accounted for without anyone walking to a shop. That tick is *not* a person's,
   * so it wears no person's colour: #131's rule is that a colour means somebody
   * claimed a thing, and a cupboard claims nothing. It goes plain stone like any
   * unattributed tick and says which jar answered for it instead, in the slot where
   * a name would otherwise sit. Unticking it is an ordinary untick — the jar was
   * empty — and re-ticking makes it yours, colour and all.
   *
   * A finished list says so and offers the next leg of the arc (#132). It is said
   * as *everything's in the kitchen* rather than in a basket: a tick means you have
   * the thing, and plenty of them were never bought — the salt was in the cupboard.
   * At the limit a list is finished the moment it opens, having never been a
   * shopping list at all, and then it says *that* instead of congratulating anyone
   * on a shop nobody did. The way onward sits *after* the list, the same place the
   * kitchen and the lobby put theirs, and the direction the page is read in. The arc
   * in the `Nav` is left exactly as it was: it draws where you have *been*, and a
   * stocked kitchen is not a cook you have done. It never blocked `cook` either, so
   * lighting that stop would remove no obstacle — it would only restate this
   * invitation further from the tick that earned it. The button carries `cook`'s
   * paprika dot instead, so the tie to the arc is said in the palette rather than by
   * moving a stop.
   */
  interface Props {
    status: BuyStatus;
    /** The consensus recipe + its ingredients, or `null` if no pick has decided. */
    recipe?: BuyRecipe | null;
    error?: string;
    /**
     * Which ingredient indices are ticked off, and where each tick came from — a
     * person, the kitchen's pantry, or nothing at all (a tap in flight, or the
     * device-local list a decision with no meal session falls back to). See
     * `Tick` in `$lib/buy`.
     */
    ticks?: Record<number, Tick>;
    onToggle?: (index: number) => void;
    /** Why the last tick did not take. Shown rather than swallowed: a line that
     * looks got but is not is how somebody comes home without the flour. */
    tickError?: string;
    /** Whether this checklist is shared with the rest of the meal, or private to
     * this device (no session to attribute a tick to). */
    shared?: boolean;
  }

  let {
    status,
    recipe,
    error,
    ticks = {},
    onToggle,
    tickError,
    shared = false,
  }: Props = $props();

  const isTicked = (i: number) => Object.hasOwn(ticks, i);

  const ticked = $derived(
    recipe ? recipe.ingredients.filter((_, i) => isTicked(i)).length : 0,
  );

  /**
   * Everything is bought (#132) — **every line of the recipe on screen is ticked,
   * and there is at least one line**.
   *
   * Both halves are load-bearing. `ticked` counts across the *current* ingredient
   * list, so a tick stranded at an index the recipe no longer has cannot finish the
   * shop, and a line the recipe has gained is simply unticked: a re-read recipe
   * reopens the list rather than inheriting a finish. And an empty list is not a
   * finished shop — a recipe with nothing to buy has its own state above and never
   * reaches here, but the `> 0` is stated anyway so nothing can ever arrive at
   * "0 of 0, done".
   *
   * It is read off exactly the same `ticks` the rows are, so it is whatever the
   * list is: the group's, live, when there is a session behind it (a peer's last
   * tick lands here through the room and finishes the shop for everyone), and this
   * device's when there is not. An in-flight tap counts, like the row it drew — if
   * the write is refused the row goes back and this goes with it, beside the reason.
   */
  const complete = $derived(
    !!recipe &&
      recipe.ingredients.length > 0 &&
      ticked === recipe.ingredients.length,
  );

  /**
   * A finished list that **nobody shopped for** (#156 × #132): every line is accounted
   * for and every one of them was already in the kitchen.
   *
   * The arithmetic of `complete` is right for this — "every line is accounted for" is
   * exactly the predicate a pre-tick satisfies honestly — but the words are not, and
   * that is the disagreement #132 left for this to settle. Congratulating a group on
   * finishing a shop none of them did is the sort of small lie that makes an app feel
   * like it is not paying attention. Measured, it is rare and real: against a
   * plausible 30-item staple pantry, 2 of the corpus's 790 recipes are born complete.
   *
   * One person's tick anywhere on the list is enough to make it a shop again, which is
   * the right threshold — somebody did go and get something.
   */
  const nothingToBuy = $derived(
    complete && Object.values(ticks).every((t) => !!t.pantry),
  );

  /** How much to get: the measured amount, or the note when there's no quantity. */
  function howMuch(ing: StructuredMeasure): string {
    return formatAmount(ing.amount) || ing.note || "";
  }

  /**
   * The row's fill. An untouched line is the paper the rest of the page is; a line
   * somebody has is washed in their tint; a line ticked with nobody behind it — the
   * pantry's, or the device-local list's — goes plain stone: got, but nobody's.
   *
   * A pantry pre-tick shares the achromatic treatment on purpose rather than getting a
   * seventh hue. Six of the palette's colours already mean "a person", and a colour
   * that meant "not a person" would be arguing with them; the absence of colour is
   * already the honest signal, and the row says *why* it is ticked in words.
   */
  function rowFill(i: number): string {
    if (!isTicked(i)) return "bg-cream-100";
    const by = ticks[i]?.by;
    return by ? userTint(by.telegram_user_id) : "bg-stone-100";
  }

  /** The checkbox itself, filled in the colour of whoever ticked it. */
  function boxAccent(i: number): string {
    const by = ticks[i]?.by;
    return by ? userAccent(by.telegram_user_id) : "accent-cocoa-500";
  }
</script>

<div class="pt-6">
  <header class="mb-6">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-plum-500" aria-hidden="true"></span>
      Buy
    </p>
    {#if status === "ready" && recipe}
      <p class="mt-1 text-sm text-stone-500">
        Everything you need for {recipe.title}.
      </p>
    {/if}
  </header>

  {#if status === "error"}
    <Alert>
      <p class="font-display text-stone-900">Couldn't load the list.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Something went wrong reaching the corpus."}
      </p>
    </Alert>
  {:else if status === "pending"}
    <ul class="flex flex-col gap-2" aria-hidden="true">
      {#each Array(8) as _, i (i)}
        <li
          class="rounded-card flex items-center gap-3 border border-stone-200 bg-cream-100 px-4 py-3"
        >
          <span class="size-5 flex-none rounded-md bg-stone-100"></span>
          <span class="rounded-pill h-4 flex-1 bg-stone-100"></span>
          <span class="rounded-pill h-4 w-16 bg-stone-100"></span>
        </li>
      {/each}
    </ul>
  {:else if !recipe}
    <Notice>
      <p class="font-display text-stone-900">Nothing to buy yet.</p>
      <p class="mt-1 text-sm text-stone-600">
        Pick something first — once the group agrees on a recipe, its ingredients
        land here.
      </p>
    </Notice>
  {:else if recipe.ingredients.length === 0}
    <Notice>
      <p class="font-display text-stone-900">{recipe.title}</p>
      <p class="mt-1 text-sm text-stone-600">No ingredients listed for it yet.</p>
    </Notice>
  {:else}
    {#if tickError}
      <div class="mb-3">
        <Alert>
          <p class="text-sm text-stone-600">{tickError}</p>
        </Alert>
      </div>
    {/if}
    <p class="mb-3 text-sm text-stone-500">
      {ticked} of {recipe.ingredients.length} in the kitchen
      {#if !shared}
        <!-- Said plainly rather than implied: a private list that looks shared is
             how two people both come home with the coriander. -->
        <span class="text-stone-400">· just on this device</span>
      {/if}
    </p>
    <ul class="flex flex-col gap-2">
      {#each recipe.ingredients as ing, i (i)}
        {@const by = ticks[i]?.by}
        {@const fromPantry = ticks[i]?.pantry}
        <li>
          <label
            class="rounded-card flex cursor-pointer items-center gap-3 border border-stone-200 px-4 py-3 {rowFill(
              i,
            )}"
          >
            <input
              type="checkbox"
              checked={isTicked(i)}
              onchange={() => onToggle?.(i)}
              class="size-5 flex-none {boxAccent(i)}"
            />
            <span
              class="font-display flex-1 {isTicked(i)
                ? 'text-stone-400 line-through'
                : 'text-stone-900'}"
            >
              {ing.item}
            </span>
            {#if by}
              <!-- Who has it. The name is always here, never only the colour:
                   six slots repeat, and not everyone separates two of them. -->
              <span class="flex-none text-sm"><UserName user={by} /></span>
            {:else if fromPantry}
              <!-- Nobody has it; the kitchen already did. Said in words and in
                   plain stone, because there is no person here to wear a colour,
                   and it names the entry so a wrong pre-tick is arguable rather
                   than mysterious. -->
              <span class="flex-none text-sm text-stone-500"
                >in the pantry · {fromPantry}</span
              >
            {/if}
            {#if howMuch(ing)}
              <span
                class="rounded-pill flex-none px-3 py-1 text-sm {isTicked(i)
                  ? 'bg-cream-50 text-stone-400'
                  : 'bg-plum-100 text-stone-600'}"
              >
                {howMuch(ing)}
              </span>
            {/if}
          </label>
        </li>
      {/each}
    </ul>

    {#if complete}
      <!-- The shop is over, so the page says so and points at the next leg.
           `role="status"` because the last tick is not always yours: in a shared
           meal it can arrive over the room, and a screen reader should hear the
           list finish rather than only find it on the next sweep. -->
      <div class="mt-6" role="status">
        <Notice>
          {#if nothingToBuy}
            <p class="font-display text-stone-900">Nothing to buy.</p>
            <p class="mt-1 text-sm text-stone-600">
              Your kitchen already has all of it.
            </p>
          {:else}
            <p class="font-display text-stone-900">
              Everything's in the kitchen.
            </p>
          {/if}
          <div class="mt-6">
            <Button href="/cook" dot="paprika">Let's cook!</Button>
          </div>
        </Notice>
      </div>
    {/if}
  {/if}
</div>
