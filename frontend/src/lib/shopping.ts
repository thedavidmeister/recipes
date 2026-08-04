import type { Voter } from "./pick";
import type { Ingredient, StructuredMeasure } from "./types";

/**
 * The shopping list's **pure half**: how a recipe becomes a list of lines, and how a
 * ticked line is held. No I/O, no `$env`, nothing that touches a network.
 *
 * That is not tidiness, it is the only place these can live. `$lib/buy` reaches the
 * API client and Turso, and both read `$env/dynamic/public` — so anything imported
 * from there **as a value** drags that read into whatever imports it. In a Storybook
 * bundle that is `undefined` and every story of the component crashes into
 * `Cannot read properties of undefined (reading 'env')`, which the visual fence
 * photographs quite happily. The unit runner has the same problem from the other end:
 * it has no SvelteKit around it on purpose (see `vitest.config.ts`), so a test that
 * imports `$lib/buy` cannot even load. Stories, tests and components take their values
 * from here; `$lib/buy` keeps the fetches.
 *
 * {@link shoppingLines} in particular is the one rule in the app that has to be
 * **identical in two languages**. The browser reads the recipe straight from Turso and
 * renders the list; the server holds the ticks, which are keyed by a line's position
 * *in this list*, and seeds the pantry pre-ticks against the same positions (#156).
 * There is no WASM to share the code through (deliberately — see CLAUDE.md), so the
 * rule is stated here and in `recipe_core::pantry::shopping_names`, and both are pinned
 * by the same case: `shopping.test.ts` here,
 * `drops_unread_lines_so_indices_match_the_shopping_list` there.
 */

/**
 * A ticked line as the screen holds it — the three ways a line can be ticked, in one
 * shape, so no surface can render two of them and forget the third.
 *
 * - `by` set → a person got it, and it wears their colour (#131). A tap of this
 *   device's own is one of these from the first paint (#210): the tapper is somebody,
 *   and this side is holding the session that says who.
 * - `pantry` set → the plan's kitchen already had it (#156). Nobody's, so no colour;
 *   it says which jar instead.
 * - both `null` → ticked with **genuinely nobody behind it**, which since #210 means
 *   one thing: a device-local tick this browser found in storage, written in some
 *   earlier sitting by whoever was at this browser then.
 *
 * Structurally identical to `BuyCheck` in `$lib/buy` minus its index, because it is the
 * same fact: keeping the two in step means the map a component reads is the row the
 * server wrote, with nothing invented in between.
 */
export interface Tick {
  by: Voter | null;
  pantry: string | null;
}

/**
 * A tick with nobody behind it — this device's stored list, whose author this browser
 * cannot name, and the honest answer for the moment before the session has been read.
 */
export const NOBODY: Tick = { by: null, pantry: null };

/**
 * A ticked line as the shared list states it: a {@link Tick}, plus the line it sits on.
 *
 * Structurally `BuyCheck` (`$lib/buy`) and `RoomBuyCheck` (`$lib/pick`) — the same row
 * read off the HTTP answer and off the room's frame — stated here so the fold below can
 * take either without importing a module that reaches `$env`.
 */
export interface TickedLine extends Tick {
  index: number;
}

/**
 * **Which lines of the shared list are ticked, and whose each tick is** (#131/#210).
 *
 * Two facts are folded, and the order between them is the whole of it. `checks` is what
 * the server last said; `inFlight` is what this device has asked for and not yet been
 * answered about. The server's word goes down first and the taps go on top, so a tap
 * shows a tick the room has not heard of yet and **never restyles one it has**.
 *
 * `you` is what stops the fold lying in the honest direction. A tap in flight used to
 * come out {@link NOBODY} — ticked, unattributed — and flip to the tapper's colour the
 * instant the room's `buy` frame landed: a colour flash on every tick anybody makes,
 * exactly at the moment of interaction. But the tapper *is* somebody, and this side
 * knows precisely who, so the tick is theirs from the first paint and the announcement
 * **confirms** what the screen already says instead of repainting it. That is #131's
 * rule applied rather than bent: a colour means somebody claimed a thing, and the
 * person who tapped claimed it.
 *
 * **The colour cannot outlive the tick**, because they are one entry: an in-flight tap
 * contributes a whole {@link Tick} or nothing at all. So whatever puts the row back
 * takes the colour with it, and there is no second piece of state to forget to clear.
 * What puts it back is the room's next whole-list frame — which the server sends for a
 * tick its own predicate refused exactly as readily as for one it took — clearing
 * `inFlight` and leaving this fold with nothing but the truth.
 *
 * `you` is `null` only while the session is still being read, and the answer then is
 * {@link NOBODY}: the truth about that moment rather than a guess at it.
 */
export function sharedTicks(
  checks: readonly TickedLine[],
  inFlight: Readonly<Record<number, boolean>>,
  you: Voter | null,
): Record<number, Tick> {
  const out: Record<number, Tick> = {};
  for (const c of checks) out[c.index] = { by: c.by, pantry: c.pantry };
  for (const [i, want] of Object.entries(inFlight)) {
    const index = Number(i);
    // `??=`, not `=`: an untick is unconditional, but a tick only fills a line the room
    // has said nothing about. A line the server has already attributed keeps that
    // attribution — to a peer, or to the pantry (#156), neither of which a tap of mine
    // may overwrite on my screen alone.
    if (want) out[index] ??= { by: you, pantry: null };
    else delete out[index];
  }
  return out;
}

/**
 * **This device's own checklist** — the solo path, for a decision with no meal session
 * behind it (`$lib/buy`'s `loadChecks`).
 *
 * `checked` is the whole list this browser holds; `mine` is the part of it the person
 * at the screen ticked in this sitting. The split is not fussiness: the stored list
 * outlives the sitting that wrote it and a browser is shared, so a tick found in
 * storage on load has no author this side can name and stays {@link NOBODY} — #131
 * again, where "somebody, probably" is not somebody. A tick made *here* has one, and
 * wears their colour from the first paint exactly as a shared tick does; there being
 * nobody else on this list makes it more certain whose it is, not less.
 *
 * Nothing here is ever rolled back. A solo tick is written to storage and answered by
 * nobody, so there is no announcement for it to disagree with.
 */
export function localTicks(
  checked: Readonly<Record<number, true>>,
  mine: Readonly<Record<number, true>>,
  you: Voter | null,
): Record<number, Tick> {
  const out: Record<number, Tick> = {};
  for (const i of Object.keys(checked)) {
    const index = Number(i);
    out[index] = mine[index] ? { by: you, pantry: null } : NOBODY;
  }
  return out;
}

/**
 * The lines the checklist shows, in the order it shows them.
 *
 * The list is the structured reading (#11), never the raw measure: the reading is what
 * `buy` renders, and a line with no reading yet is dropped rather than shown raw
 * (`pick` serves read recipes, so a decided one carries readings throughout). Because a
 * dropped line takes no index, this filter *is* the index — a disagreement about which
 * lines count is a disagreement about which row a tick belongs to.
 */
export function shoppingLines(ingredients: Ingredient[]): StructuredMeasure[] {
  return ingredients
    .map((i) => i.structured)
    .filter(
      (s): s is StructuredMeasure => !!s && !!s.item && s.item.trim() !== "",
    );
}
