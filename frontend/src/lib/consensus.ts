import type { Match, RecipeCard } from "./types";

/**
 * A pick's **win condition** (#20): how the running tally is keyed, how many a recipe
 * has to win over, and which recipes have won.
 *
 * Pure — no I/O, no `$env`, nothing reactive — so vitest and a story can import it
 * while the page keeps the socket, the deck and the navigation (the `$lib/shopping`
 * split, which `lint:env` enforces).
 *
 * **One number decides, and it is the roster (#181).** A pick has two counts in the
 * air and they are not the same fact:
 *
 * - `ServerMsg::Lobby.deciders` — the plan's roster: who joined, and so who a recipe
 *   has to win over. The server sends it on connect and again on every roster change,
 *   so the count the page holds is the server's, not a tally the client kept.
 * - `ServerMsg::Tally.participants` — `COUNT(DISTINCT voter_id) FROM votes`, which is
 *   how many people have swiped **at all**. It is not a roster and must never decide:
 *   the host's very first yes arrives as `participants: 1, yes: 1, no: 0`, unanimous
 *   by that arithmetic, and a decision is stashed and navigated to off one swipe of
 *   one card while everyone else is still looking at their first recipe.
 *
 * So the deciding count has exactly one producer — {@link decidingCount}, off the
 * roster — and {@link agreed} accepts nothing else. That is what {@link Deciding} is
 * for: a plain `number` parameter would take the tally's voter count just as happily,
 * and the whole of #181 is that the two numbers had stopped being told apart.
 */

/**
 * The number a recipe has to win over.
 *
 * Branded so it can only come out of {@link decidingCount}. Both counts in a pick are
 * `number`, so nothing but the type keeps the wrong one out of {@link agreed}, and
 * the cost of mixing them up is a group sent to `/buy` for a recipe it never agreed
 * on.
 */
export type Deciding = number & { readonly __deciding: unique symbol };

/**
 * Encode the `(source, id)` of a recipe unambiguously — the key every tally, card and
 * vote is held under.
 *
 * A bare `${source}:${id}` would collide if a future source or id ever held a colon,
 * silently merging two recipes' tallies: one recipe's yeses counted toward another's
 * consensus is a wrong dinner, not a display glitch.
 */
export function cardKey(source: string, id: string): string {
  return JSON.stringify([source, id]);
}

/**
 * How many a recipe has to win over, from the plan's roster — or `undefined` while
 * nothing has told us the roster yet.
 *
 * **Unknown is not one.** A client opening its socket is sent the tally *before* the
 * lobby, so there is a frame in between where the votes are known and the roster is
 * not, and a fresh page has neither. Reading that gap as "one decider" makes any
 * single yes already in the tally unanimous — and the decision is sticky: it stashes
 * the recipe and navigates to `/buy`. Absent stays absent, and {@link agreed} answers
 * nothing at all until the roster arrives.
 *
 * **The floor is one.** Your own yes is unanimous when you are the only one in the
 * plan, so a roster that counts nobody still takes one yes to win rather than none.
 */
export function decidingCount(
  roster: number | undefined,
): Deciding | undefined {
  return roster === undefined ? undefined : (Math.max(roster, 1) as Deciding);
}

/**
 * The recipes everyone deciding said yes to and nobody said no to — a pick's whole
 * point (#20) — in the order the tally ranked them, so the first is the one a pick
 * decides on.
 *
 * Three things keep a wrong answer out, and each is doing its own job:
 *
 * - **An unknown roster agrees to nothing.** Not "one", not "everyone so far".
 * - **A no is a veto**, even against a full house. A live `vote` frame adds a yes
 *   without taking back the no it replaced (the next tally does), so the two counts
 *   can briefly both be set for one person; the veto stands until the tally settles
 *   it, because holding a decision back is recoverable and making one is not.
 * - **A recipe whose card has not arrived is not a match.** The tally names recipes
 *   this client has never walked to, and their cards are fetched after the fact —
 *   there is nothing to decide *on* until one is here.
 */
export function agreed(
  yes: Record<string, number>,
  no: Record<string, number>,
  deciding: Deciding | undefined,
  cards: Record<string, RecipeCard>,
): Match[] {
  if (deciding === undefined) return [];
  return Object.keys(yes)
    .filter((k) => (yes[k] ?? 0) === deciding && (no[k] ?? 0) === 0)
    .map((k) => {
      const card = cards[k];
      return card ? { card, yes: yes[k] ?? 0 } : null;
    })
    .filter((m): m is Match => m !== null);
}
