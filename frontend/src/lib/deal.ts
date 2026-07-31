/**
 * When a swiper has run out of cards, which of the two reasons it is — and what to say
 * about it (#202).
 *
 * A pick used to have only one answer to an empty deck: *"Finding more recipes…"*. That
 * is true while a plan still holds recipes this person has not seen, and it is a lie the
 * moment they have answered every one — which a plan that runs for days reaches, because
 * the deal now skips what you have already voted on. Waiting on the others is a real
 * state, not an error and not a slow network, and it says so.
 *
 * Pure and free of `$env`, so a story and a unit test can both import it — the same split
 * `$lib/shopping` is on, and the reason this does not live in `$lib/pick` (which reaches
 * the network client).
 */

/**
 * Has this member answered everything the plan can currently deal them?
 *
 * `dealt` is how many stops the walk returned, **not** how many were new to this client's
 * deck. The distinction is the whole correctness of this: the server's deal already
 * excludes the recipes this caller has voted on in this plan, so a deal that comes back
 * empty is the plan saying there is nothing left for them. A deal full of cards this
 * client happens to hold already is a *client* still holding cards, and answers nothing
 * about the member.
 *
 * It is a question about the last deal, never a flag that gets set. The deal is
 * recomputed on every walk, so a recipe becoming dealable mid-plan — the meal-time
 * worker reading one the round can serve (#193) — un-finishes this on the next refill
 * with nothing to invalidate.
 */
export function answeredEverything(dealt: number): boolean {
  return dealt === 0;
}

/**
 * The line under "You've answered everything." — who is left to hear from.
 *
 * `deciding` is the roster: how many people this plan needs agreement from, this member
 * included. So the number said out loud is the *others*, which is one fewer.
 *
 * Nobody else is a real arrival and gets its own sentence rather than "0 others are still
 * deciding". A solo swiper who has answered everything is not waiting on anyone — the
 * roster closed at the start (#96/#169), so no one is coming — and telling them to hang
 * on for zero people would be the same dishonest holding pattern this replaces.
 */
export function waitingOnOthers(deciding: number): string {
  const others = Math.max(deciding - 1, 0);
  if (others === 0) return "Just you in this plan.";
  if (others === 1) return "1 other is still deciding.";
  return `${others} others are still deciding.`;
}
