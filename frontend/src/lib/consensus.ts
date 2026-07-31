/**
 * A pick's **win condition** (#20): how the running tally is keyed, and how many a
 * recipe has to win over.
 *
 * Pure — no I/O, no `$env`, nothing reactive — so vitest and a story can import it
 * while the page keeps the socket, the deck and the navigation (the `$lib/shopping`
 * split, which `lint:env` enforces).
 *
 * **Who answers it, and where that moved.** #181 established that one number decides
 * and it is the roster, not the tally's voter count. #201 moved the answering itself:
 * `agreed` used to live here, taking that count and naming the recipes everyone had
 * said yes to, and it is **gone**. The server evaluates the condition inside the
 * vote's own write, records it on `pick_sessions`, and broadcasts it
 * (`ServerMsg::Decided`, surfaced as `PickHandlers.onDecided`). Two evaluators of one
 * condition are two answers to "what did we pick" — which is exactly the duplication
 * that let a browser name any recipe it liked to `buy` — so there is one, and it is
 * the side holding the roster and the votes.
 *
 * What stays is the part that was never the decision: the tally's key, and the count
 * the pick *shows*. Both still deserve their tests, because both still have a wrong
 * answer that is silent.
 *
 * A pick has two counts in the air and they are still not the same fact:
 *
 * - `ServerMsg::Lobby.deciders` — the plan's roster: who joined, and so who a recipe
 *   has to win over. The server sends it on connect and again on every roster change,
 *   so the count the page holds is the server's, not a tally the client kept.
 * - `ServerMsg::Tally.participants` — `COUNT(DISTINCT voter_id) FROM votes`, which is
 *   how many people have swiped **at all**. One person swiping once makes it 1,
 *   whoever else is in the plan.
 *
 * So the deciding count still has exactly one producer — {@link decidingCount}, off
 * the roster — and {@link Deciding} still brands its result. Nothing downstream of it
 * ends a pick any more, but a caption that tells a room of three that one person is
 * deciding is its own wrong answer, and the brand is what keeps the two numbers told
 * apart at all.
 */

/**
 * The number a recipe has to win over — and, since #201, the number the pick shows for
 * it rather than the number it measures against.
 *
 * Branded so it can only come out of {@link decidingCount}. Both counts in a pick are
 * `number`, so nothing but the type keeps the tally's voter count from standing in for
 * the roster; #181 is the record of what happens when they stop being told apart.
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
 * not, and a fresh page has neither. Reading that gap as "one decider" is what #181
 * was: it made any single yes already in the tally unanimous, and the decision was
 * sticky — it stashed the recipe and navigated to `/buy`. Since #201 that gap can no
 * longer end a pick, and it is still not one decider: absent stays absent, so a page
 * that has not been told the roster says nothing about it rather than guessing.
 *
 * **The floor is one.** Your own yes is unanimous when you are the only one in the
 * plan, so a roster that counts nobody still takes one yes to win rather than none —
 * the same arithmetic the server's own `EXISTS (roster)` clause refuses to do without.
 */
export function decidingCount(
  roster: number | undefined,
): Deciding | undefined {
  return roster === undefined ? undefined : (Math.max(roster, 1) as Deciding);
}
