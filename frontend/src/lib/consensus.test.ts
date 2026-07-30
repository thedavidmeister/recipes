import { describe, expect, it } from "vitest";
import { agreed, cardKey, decidingCount } from "./consensus";
import { recipeCards } from "./fixtures";
import type { RecipeCard } from "./types";

/**
 * The pick's win condition, pinned (#181).
 *
 * This is the one derivation in the app that ends a pick: reach it and the recipe is
 * stashed and the whole group is navigated to `/buy`, with no undo. Every case here
 * is therefore about **not** reaching it wrongly — the expected values come from the
 * protocol (`session::ServerMsg`) rather than from the implementation, because the
 * two counts a pick receives mean different things on the wire:
 *
 * - `Lobby.deciders` is `view.voters.len()` — the roster, who a recipe must win over.
 * - `Tally.participants` is `COUNT(DISTINCT voter_id) FROM votes` — who has swiped at
 *   all. One person swiping once makes it 1, whoever else is in the plan.
 *
 * The bug this file exists to keep out is deciding with the second, or with a roster
 * nothing has stated yet: both read a lone yes as unanimous.
 */

const [alpha, beta, gamma] = recipeCards();

/** The tally's key for a card — the same one the page holds its cardMap under. */
const k = (c: RecipeCard) => cardKey(c.source, c.id);

/** The cards this client has actually fetched, as the page holds them. */
function held(...cs: RecipeCard[]): Record<string, RecipeCard> {
  return Object.fromEntries(cs.map((c) => [k(c), c]));
}

describe("cardKey", () => {
  it("cannot merge two recipes' tallies over a punctuation mark", () => {
    // `${source}:${id}` reads both of these as "a:b:c", and one recipe's yeses
    // counted toward another's consensus is a wrong dinner, not a display glitch.
    expect(cardKey("a:b", "c")).not.toBe(cardKey("a", "b:c"));
  });
});

describe("decidingCount", () => {
  it("has no count at all until something states the roster", () => {
    // Not 0, and emphatically not 1: a client that has votes but no roster yet must
    // be unable to answer, not answer "one".
    expect(decidingCount(undefined)).toBeUndefined();
  });

  it("is the roster, so a recipe wins over everyone who joined", () => {
    // Straight off `Lobby.deciders`; nobody has to have voted for it to be three.
    expect(decidingCount(3)).toBe(3);
    expect(decidingCount(2)).toBe(2);
  });

  it("floors at one, so your own yes is unanimous when you are alone", () => {
    expect(decidingCount(1)).toBe(1);
    // A roster that counts nobody still takes one yes to win, never zero — at zero
    // every recipe in the tally with no votes against it would be "agreed".
    expect(decidingCount(0)).toBe(1);
  });
});

describe("agreed", () => {
  it("agrees nothing while the roster is unknown, however the tally reads", () => {
    // The exact frame order a fresh socket gets: the tally lands first, carrying
    // whatever has been swiped so far. One yes and no noes is what `Tally` looks
    // like after a single swipe, and it must not end the pick.
    expect(
      agreed({ [k(alpha)]: 1 }, { [k(alpha)]: 0 }, undefined, held(alpha)),
    ).toEqual([]);
    // Nor does a bigger tally, which is what a reload mid-pick rehydrates into.
    expect(
      agreed({ [k(alpha)]: 3 }, { [k(alpha)]: 0 }, undefined, held(alpha)),
    ).toEqual([]);
  });

  it("names the recipe every decider said yes to", () => {
    expect(
      agreed(
        { [k(alpha)]: 3, [k(beta)]: 1 },
        { [k(alpha)]: 0, [k(beta)]: 0 },
        decidingCount(3),
        held(alpha, beta),
      ),
    ).toEqual([{ card: alpha, yes: 3 }]);
  });

  it("leaves a recipe one yes short of the roster undecided", () => {
    // Two of three: the third has not swiped it yet, which is not the same as
    // agreeing, and is exactly what deciding on the voter count would call a win.
    expect(
      agreed(
        { [k(alpha)]: 2 },
        { [k(alpha)]: 0 },
        decidingCount(3),
        held(alpha),
      ),
    ).toEqual([]);
  });

  it("lets one no veto a recipe the count would otherwise call agreed", () => {
    // A live `vote` frame adds the yes without taking back the no it replaced, so
    // both counts can be set for one person until the next tally settles it. Holding
    // a decision back is recoverable; making one is not.
    expect(
      agreed(
        { [k(alpha)]: 3 },
        { [k(alpha)]: 1 },
        decidingCount(3),
        held(alpha),
      ),
    ).toEqual([]);
  });

  it("holds back a recipe whose card has not arrived yet", () => {
    // The tally names recipes this client never walked to; their cards are fetched
    // afterwards, and there is nothing to decide on until one is here.
    const yes = { [k(gamma)]: 2 };
    const no = { [k(gamma)]: 0 };
    expect(agreed(yes, no, decidingCount(2), held(alpha))).toEqual([]);
    expect(agreed(yes, no, decidingCount(2), held(alpha, gamma))).toEqual([
      { card: gamma, yes: 2 },
    ]);
  });

  it("keeps the tally's ranking, so the first match is the one a pick takes", () => {
    // `load_tally` orders by `yes DESC, no ASC`, and the page decides on the first.
    expect(
      agreed(
        { [k(beta)]: 2, [k(alpha)]: 2 },
        { [k(beta)]: 0, [k(alpha)]: 0 },
        decidingCount(2),
        held(alpha, beta),
      )[0],
    ).toEqual({ card: beta, yes: 2 });
  });
});

describe("the frames a client is actually sent", () => {
  it("does not decide on the tally that arrives before the lobby", () => {
    // A socket rehydrates in this order: `Tally`, then `Lobby` (session.rs's
    // socket_loop sends them in that order, as two frames). Alice has swiped one
    // yes; bob and carol are also in the plan and have swiped nothing.
    const yes = { [k(alpha)]: 1 };
    const no = { [k(alpha)]: 0 };
    let roster: number | undefined;

    // Frame 1 — the tally. `participants` would be 1 here; the roster is unstated.
    expect(agreed(yes, no, decidingCount(roster), held(alpha))).toEqual([]);

    // Frame 2 — the lobby names the roster, and one of three is still one of three.
    roster = 3;
    expect(agreed(yes, no, decidingCount(roster), held(alpha))).toEqual([]);

    // And the pick ends when the other two agree, not before.
    expect(
      agreed({ [k(alpha)]: 3 }, no, decidingCount(roster), held(alpha)),
    ).toEqual([{ card: alpha, yes: 3 }]);
  });

  it("carries the roster across a dropped socket rather than forgetting it", () => {
    // The roster is frozen at the start in both directions (#96/#169) and no vote
    // exists before the start (#175/#179), so the count held across a drop is the
    // count the plan started with. Nothing arrives during the gap, so nothing can
    // decide in it; the reconnect replaces the tally and decides on the same three.
    const roster = 3;
    const no = { [k(alpha)]: 0 };
    expect(
      agreed({ [k(alpha)]: 2 }, no, decidingCount(roster), held(alpha)),
    ).toEqual([]);
    expect(
      agreed({ [k(alpha)]: 3 }, no, decidingCount(roster), held(alpha)),
    ).toEqual([{ card: alpha, yes: 3 }]);
  });
});
