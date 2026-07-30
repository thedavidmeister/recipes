import type { Voter } from "./pick";

/**
 * Who is deciding a meal plan, and who is only watching it (#180).
 *
 * A plan's roster closes the moment the swiping begins (#93/#96): from then on the
 * people on it are the people a recipe has to win over, and anybody else holding the
 * link can see the plan but not vote in it. #179 made that true where it counts — a
 * vote from outside the roster is refused by `record_vote`'s own predicate — and this
 * is the half that says so on screen, because the refusal cannot say it itself: a
 * vote travels as a socket frame the server never answers (`socket_loop` records
 * "the socket has nowhere to report either"), so a swipe that lands nowhere produces
 * no error to render. There is nothing to catch and nothing to show *after* the fact,
 * which is exactly why watching has to be stated *before* it.
 *
 * Pure, and its own module rather than a corner of `$lib/pick`, for the
 * `$lib/shopping` reason (#176/7): `$lib/pick` reaches `$lib/client`, which reads
 * `$env/dynamic/public`, so a value imported from it drags that read into Storybook
 * and the unit runner, where it is `undefined`. This is display state, so it lives on
 * the side of the split that stories and tests can import. `lint:env` enforces it.
 */

/**
 * Whether this viewer holds a seat — the roster is the only thing asked.
 *
 * **Not when they arrived.** A person the host seated before the start (#72) who
 * opens the link an hour into the swiping is on the roster, so they are deciding;
 * `join_lobby` re-seats them without complaint for the same reason (its refusal is
 * `started && !on the roster`, never `started` alone). Arrival time is not a fact the
 * client holds, the server does not ask it, and collapsing the two states would take
 * the vote away from somebody who has a seat.
 *
 * An unknown viewer — the session query has not resolved — is not seated *and* not a
 * watcher: see [`isWatching`].
 */
export function isDecider(
  roster: Voter[] | undefined,
  viewer: string | undefined,
): boolean {
  if (!roster || !viewer) return false;
  return roster.some((v) => v.telegram_user_id === viewer);
}

/**
 * Whether this viewer is watching a plan rather than deciding it.
 *
 * Three things have to be true together, and each is asked for its own reason:
 *
 * - **the plan has started.** Before that there is no watching to do — the lobby
 *   seats anyone who opens the link, so a viewer who is not on the roster yet is a
 *   moment away from being on it.
 * - **the roster has been read.** Undefined is *not yet known*, which is a different
 *   thing from *does not contain you*; the socket can set `started` from a lobby
 *   frame before the HTTP read lands.
 * - **we know who is looking.** Same rule from the other side. Telling someone they
 *   are only watching is a claim about them, so it waits until there is somebody to
 *   make it about — the session is a cache read the layout already paid for, so this
 *   is a frame, not a wait.
 */
export function isWatching(plan: {
  started: boolean | undefined;
  roster: Voter[] | undefined;
  viewer: string | undefined;
}): boolean {
  if (plan.started !== true) return false;
  if (!plan.roster || !plan.viewer) return false;
  return !isDecider(plan.roster, plan.viewer);
}
