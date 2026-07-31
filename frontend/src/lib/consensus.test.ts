import { describe, expect, it } from "vitest";
import { cardKey, decidingCount } from "./consensus";

/**
 * The pick's tally key and its deciding count, pinned (#181, #201).
 *
 * This file used to pin `agreed` as well — the one derivation in the app that ended a
 * pick — and that function is gone. #201 moved the win condition to the server, where
 * it is evaluated inside the vote's own write and recorded on the plan, so a client no
 * longer reaches the conclusion at all: it is handed one (`ServerMsg::Decided`, and
 * `frames.test.ts` is where *that* is pinned). The cases those tests protected are now
 * `backend/src/session.rs`'s — `a_roster_one_yes_short_decides_nothing`,
 * `one_no_holds_back_a_recipe_everyone_else_wanted`, and the rest — asked of the side
 * that holds the roster and the votes.
 *
 * What is left is not the decision, and is still worth pinning, because both halves
 * fail silently:
 *
 * - `cardKey` keys the tally, the cards and the votes. Collide two recipes and one's
 *   yeses land under the other's name — a wrong count on screen, and a wrong card
 *   pulled into a deck.
 * - `decidingCount` is what the pick *says* is deciding. The two counts a client
 *   receives still mean different things — `Lobby.deciders` is the roster,
 *   `Tally.participants` is who has swiped at all — and a caption that tells a room of
 *   three that one person is deciding is its own wrong answer, even though it can no
 *   longer end anything.
 */

describe("cardKey", () => {
  it("cannot merge two recipes' tallies over a punctuation mark", () => {
    // `${source}:${id}` reads both of these as "a:b:c", and one recipe's yeses
    // counted toward another's is a wrong count, not a display glitch.
    expect(cardKey("a:b", "c")).not.toBe(cardKey("a", "b:c"));
  });
});

describe("decidingCount", () => {
  it("has no count at all until something states the roster", () => {
    // Not 0, and emphatically not 1. A socket rehydrates `Tally` before `Lobby`, so
    // there is a frame where the votes are known and the roster is not; answering
    // "one" in it was #181, and it is still not something to say out loud.
    expect(decidingCount(undefined)).toBeUndefined();
  });

  it("is the roster, so it counts everyone who joined", () => {
    // Straight off `Lobby.deciders`; nobody has to have voted for it to be three.
    expect(decidingCount(3)).toBe(3);
    expect(decidingCount(2)).toBe(2);
  });

  it("floors at one, because you are deciding when you are alone", () => {
    expect(decidingCount(1)).toBe(1);
    // A roster that counts nobody still reads as one person deciding rather than
    // none — the same arithmetic the server refuses to do without its own
    // `EXISTS (roster)` clause, where zero deciders would agree to everything.
    expect(decidingCount(0)).toBe(1);
  });
});
