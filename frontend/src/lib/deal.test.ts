import { describe, expect, it } from "vitest";
import { answeredEverything, waitingOnOthers } from "./deal";

describe("answeredEverything", () => {
  it("says so when the deal comes back with nothing in it", () => {
    // The server already dropped what this caller has voted on in this plan (#202),
    // so an empty deal is the plan saying there is nothing left for them.
    expect(answeredEverything(0)).toBe(true);
  });

  it("says nothing of the sort while the deal still holds cards", () => {
    expect(answeredEverything(1)).toBe(false);
    expect(answeredEverything(30)).toBe(false);
  });

  it("un-finishes the moment a later deal has something in it", () => {
    // The finished state is a reading of the *last* deal, never a flag that latches.
    // A recipe becoming dealable mid-plan — the meal-time worker reading one this
    // round can serve (#193) — puts a card back in the next deal, and this has to
    // follow it back without anything invalidating anything.
    expect(answeredEverything(0)).toBe(true);
    expect(answeredEverything(1)).toBe(false);
  });

  it("does not read a deck this client already holds as a finished member", () => {
    // A deal of 30 stops the client has all queued adds nothing to the deck, and says
    // nothing about whether the member has answered them. Only what the *deal* held
    // counts; conflating the two would call a busy client finished.
    expect(answeredEverything(30)).toBe(false);
  });
});

describe("waitingOnOthers", () => {
  it("counts the others, not the roster", () => {
    expect(waitingOnOthers(3)).toBe("2 others are still deciding.");
    expect(waitingOnOthers(5)).toBe("4 others are still deciding.");
  });

  it("says one other in the singular", () => {
    expect(waitingOnOthers(2)).toBe("1 other is still deciding.");
  });

  it("never claims nought others are deciding", () => {
    // A solo swiper who has answered everything is waiting on nobody — the roster
    // closed at the start (#96/#169), so no one is coming. "0 others are still
    // deciding" would be the same dishonest holding pattern this state replaces.
    expect(waitingOnOthers(1)).toBe("Just you in this plan.");
  });

  it("cannot be talked below nobody", () => {
    // The roster floors at one on the way in, but a count of zero must not come out
    // as "-1 others".
    expect(waitingOnOthers(0)).toBe("Just you in this plan.");
  });
});
