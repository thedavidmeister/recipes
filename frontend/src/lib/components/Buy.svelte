<script lang="ts">
  import Alert from "./Alert.svelte";
  import Button from "./Button.svelte";
  import Notice from "./Notice.svelte";
  import UserName from "./UserName.svelte";
  import type { Tick } from "$lib/shopping";
  import type { Voter } from "$lib/pick";
  import type { BuyRecipe, BuyStatus, StructuredMeasure } from "$lib/types";
  import { userAccent, userColour, userTint } from "$lib/colour";
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
   * **A watcher sees the list and taps nothing** (#200/#222). Somebody who opened a
   * plan that started without them is on no roster, so the server refuses every write
   * they could make from here; this screen therefore states each tick instead of
   * offering it, keeps every attribution, and says whose shop it is in the footer. It
   * is `cook`'s arrangement — the page that already knew — brought to the one surface
   * that never learned it.
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
     * person, the kitchen's pantry, or nobody at all (a tick this browser restored
     * from the device-local list a decision with no meal session falls back to). A
     * tap made on this screen arrives here already the tapper's (#210), so nothing in
     * here is ever "ticked, owner pending". See `Tick` in `$lib/shopping`.
     */
    ticks?: Record<number, Tick>;
    onToggle?: (index: number) => void;
    /** Why the last tick did not take. Shown rather than swallowed: a line that
     * looks got but is not is how somebody comes home without the flour. */
    tickError?: string;
    /** Whether this checklist is shared with the rest of the meal, or private to
     * this device (no session to attribute a tick to). */
    shared?: boolean;
    /**
     * Watching the plan rather than shopping it (#200/#222) — the whole list is
     * visible, no control is offered.
     *
     * `cook`'s arrangement, on the surface that never learned it. A watcher sees every
     * line, every tick and whose each one is, because that is what watching **is**;
     * what they do not get is a box to tap, since the server refuses their tick at the
     * framework's guard (`events::Guard::SeatedInStartedPlan`). Absent, not disabled —
     * a greyed-out checkbox still offers itself and still explains nothing — and the
     * footer says whose shop it is instead, which is `pick`'s pattern for the same
     * sentence.
     *
     * The ticked state stays on screen as a **stated fact**: a filled square in the
     * colour of whoever got it, where the tappable box was. Cook does the same thing
     * with a timed step's duration — how long it takes is the recipe's fact, and only
     * starting it is somebody's to do.
     *
     * Never true on a device-local list: there is no plan, so there is nobody to be
     * watching (`isWatching` asks the roster, and a solo shop has none).
     */
    watching?: boolean;
    /**
     * Who is shopping — the plan's roster, for the watching line to name.
     *
     * Only read while `watching`, exactly as `Pick` reads its own: a shopper is one of
     * these people and does not need to be told who the others are, and the page hands
     * over an empty list rather than a `null` when the lobby has not been read.
     */
    roster?: Voter[];
    /**
     * Start the room's cook (#211) — raised as a session event, so the whole plan comes
     * along.
     *
     * Called only on a `shared` list a watcher is not looking at; a solo one has nobody
     * to bring and its button is the plain link it always was, and a watcher is offered
     * no button at all (#200 — every write from this screen is refused, so a control
     * that looked pressable would be a control that did nothing without saying why).
     * The page navigates on the room's announcement rather than on this call.
     */
    onCook?: () => void;
  }

  let {
    status,
    recipe,
    error,
    ticks = {},
    onToggle,
    tickError,
    shared = false,
    watching = false,
    roster = [],
    onCook,
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
   * somebody has is washed in their tint — including one this screen has only just
   * tapped, which is somebody's the moment it is tapped (#210); a line ticked with
   * nobody behind it — the pantry's, or a tick restored from this device's own list —
   * goes plain stone: got, but nobody's.
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

  /**
   * The same box, for a watcher: the tick as a **stated fact** rather than a control
   * (#222).
   *
   * It says what the native checkbox says and offers nothing: a solid square in the
   * colour of whoever got the line — the `-500` weight the browser draws inside the
   * accent, which is the one place a person's colour is already allowed to be a fill —
   * an unattributed tick in cocoa like its accent, and an empty outline for a line
   * nobody has. Fill against outline is the achromatic half, so the two palest slots say
   * "got" as clearly as the darkest, and the row's strike-through, its name and the
   * counter above say it again.
   */
  function statedBox(i: number): string {
    if (!isTicked(i)) return "border border-stone-300";
    const by = ticks[i]?.by;
    return by ? userColour(by.telegram_user_id) : "bg-cocoa-500";
  }
</script>

<div class="pt-6">
  <header class="mb-6">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="bg-plum-500 size-2.5 rounded-full" aria-hidden="true"></span>
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
        {error ?? "Try again in a moment."}
      </p>
    </Alert>
  {:else if status === "pending"}
    <ul class="flex flex-col gap-2" aria-hidden="true">
      {#each Array(8) as _, i (i)}
        <li
          class="rounded-card bg-cream-100 flex items-center gap-3 border border-stone-200 px-4 py-3"
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
        Pick something first — once the group agrees on a recipe, its
        ingredients land here.
      </p>
    </Notice>
  {:else if recipe.ingredients.length === 0}
    <Notice>
      <p class="font-display text-stone-900">{recipe.title}</p>
      <p class="mt-1 text-sm text-stone-600">
        No ingredients listed for it yet.
      </p>
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
        <li>
          {#if watching}
            <!-- A watcher's row: everything the shopper's row says, with the box
                 stated instead of offered. A `div` and not a `label`, because a
                 label round nothing is a click target that leads nowhere. -->
            <div
              class="rounded-card flex items-center gap-3 border border-stone-200 px-4 py-3 {rowFill(
                i,
              )}"
            >
              <span
                class="size-5 flex-none rounded-md {statedBox(i)}"
                aria-hidden="true"
              ></span>
              {@render line(ing, i)}
            </div>
          {:else}
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
              {@render line(ing, i)}
            </label>
          {/if}
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
          {#if !watching}
            <!-- Absent for a watcher, not disabled — `pick`'s rule about the Yes
                 button, one page along. Starting the room's cook is refused at the
                 server's guard (`SeatedInDecidedPlan`), so a button here would offer
                 itself and explain nothing; the footer below says whose plan it is
                 instead, once, for this and for the boxes. A watcher is not left
                 behind by its absence — the room's `cooking` frame reaches every
                 socket, so they are carried to the stove by the announcement exactly
                 as they were carried here by the decision. -->
            <div class="mt-6">
              {#if shared}
                <!-- Starting the cook is a thing that happens to the *meal*, not a place
                     this browser goes: it is raised on the plan's room, and every screen
                     in it — the tapper's included — moves when the room's announcement
                     comes back. So the control is a button and not a link, which is also
                     what it now honestly is: following an href would take one person to
                     the stove and leave the rest holding the list, which is the whole
                     bug. -->
                <Button onclick={onCook} dot="paprika">Let's cook!</Button>
              {:else}
                <!-- Nobody to bring, so nothing to raise: a list with no meal session
                     behind it keeps the plain link it has always had. -->
                <Button href="/cook" dot="paprika">Let's cook!</Button>
              {/if}
            </div>
          {/if}
        </Notice>
      </div>
    {/if}

    {#if watching}
      <!-- The line that says whose shop this is — `Pick`'s footer, one page along,
           because watching means one thing everywhere. It sits under the whole list
           rather than in any one row the box left, so it is one sentence for every
           absent control above it instead of sixteen apologies. Each name wears its
           own colour, the app's one device for whose a thing is, and the sentence
           around them wears the resting voice every other quiet line here wears. -->
      <footer class="mt-6 border-t border-stone-200 pt-4">
        <p class="text-sm text-stone-500">
          {#if roster.length}
            <!-- Each name keeps its own separator so the punctuation cannot drift
                 away from the name it belongs to, and the spacing is in the string
                 rather than between the tags: Svelte trims the whitespace at an each
                 block's edges. -->
            {#each roster as v, i (v.telegram_user_id)}
              <span
                ><UserName user={v} inline />{i < roster.length - 2
                  ? ", "
                  : i === roster.length - 2
                    ? " and "
                    : ""}</span
              >
            {/each}
            {roster.length === 1 ? "is" : "are"} shopping.
          {:else}
            <!-- The roster has not been read. The plan is still somebody's, and
                 saying so without names beats saying nothing. -->
            This plan is somebody else's shop.
          {/if}
          You're watching — this plan started without you.
        </p>
      </footer>
    {/if}
  {/if}
</div>

<!-- One line of the list, without its box: what to get, who has it, and how much. The
     shopper's row and the watcher's row differ **only** in the control at the front, so
     the rest of the line is written once and neither version can drift from the other —
     a watcher who could not see an attribution would not be watching the same list. -->
{#snippet line(ing: StructuredMeasure, i: number)}
  {@const by = ticks[i]?.by}
  {@const fromPantry = ticks[i]?.pantry}
  <span
    class="font-display flex-1 {isTicked(i)
      ? 'text-stone-400 line-through'
      : 'text-stone-900'}"
  >
    {ing.item}
  </span>
  {#if by}
    <!-- Who has it. The name is always here, never only the colour: six slots
         repeat, and not everyone separates two of them. -->
    <span class="flex-none text-sm"><UserName user={by} /></span>
  {:else if fromPantry}
    <!-- Nobody has it; the kitchen already did. Said in words and in plain stone,
         because there is no person here to wear a colour, and it names the entry so
         a wrong pre-tick is arguable rather than mysterious. -->
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
{/snippet}
