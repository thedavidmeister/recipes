import { describe, expect, it } from "vitest";
import { localTicks, NOBODY, sharedTicks, shoppingLines } from "./shopping";
import type { Voter } from "./pick";
import type { Ingredient } from "./types";

/**
 * The shopping list's pure half, pinned: which lines it shows, and **whose** each
 * ticked one is.
 *
 * ---
 *
 * The **indexing** first.
 *
 * `buy_checks` is keyed by a line's position in this list, so this projection decides
 * which row a tick lands on. It is stated twice — here for the browser, which reads the
 * recipe straight from Turso, and in `recipe_core::pantry::shopping_names` for the
 * server, which holds the ticks and seeds the pantry pre-ticks (#156). There is no
 * third place to put it (no WASM, deliberately — see CLAUDE.md), so the two are held
 * together by the same case being asserted on both sides:
 * `drops_unread_lines_so_indices_match_the_shopping_list` in `crates/recipe-core`
 * asserts the same fixture, name for name, index for index.
 *
 * If these two ever drift, a pre-tick lands on the wrong line and somebody is told
 * they already have something they do not.
 *
 * ---
 *
 * Then the **attribution** (#131/#210), which is why the two folds below exist at all.
 * A story cannot pin it: a story is handed a finished `ticks` map, so a render proves
 * what an attributed row *looks like*, never which rows the page calls whose. The whole
 * of #210 is one `??=` and one `you`, and the two failures either could produce — a
 * flash back to nobody, or a colour left on a line that has no tick — are silent in
 * production, because a refused tick travels over a socket the server never answers on.
 */

function ing(name: string, item?: string): Ingredient {
  return {
    name,
    measure: null,
    structured:
      item === undefined
        ? null
        : { item, amount: null, preparation: null, note: null },
  };
}

describe("shoppingLines", () => {
  it("indexes only the lines the checklist shows", () => {
    // The same five lines the Rust test uses, in the same order.
    const lines = shoppingLines([
      ing("2 large onions", "onions"),
      ing("a splash of something"),
      ing("salt", "salt"),
      ing("mystery", "   "),
      ing("1 tbsp olive oil", "olive oil"),
    ]);
    expect(lines.map((l) => l.item)).toEqual(["onions", "salt", "olive oil"]);
    // "salt" is index 1 on the list, not index 2 in the recipe. That shift is the
    // whole reason this is pinned on both sides.
    expect(lines[1].item).toBe("salt");
  });

  it("drops a line with no reading rather than showing it raw", () => {
    // `pick` serves read recipes, so this is a corpus mid-enrichment, not a normal
    // state — but it must not put a nameless row on a shopping list either way.
    expect(shoppingLines([ing("1 (14 oz) can of something")])).toEqual([]);
  });

  it("treats a whitespace-only name as no name", () => {
    // A reading whose item is blank would render as an unlabelled tickbox, and would
    // take an index, pushing every line after it onto the wrong tick.
    expect(
      shoppingLines([ing("x", ""), ing("y", " \t "), ing("z", "z")]),
    ).toEqual([{ item: "z", amount: null, preparation: null, note: null }]);
  });

  it("keeps an unread recipe's list empty rather than inventing lines", () => {
    expect(shoppingLines([])).toEqual([]);
  });
});

/** The person at this screen, and somebody else in the same meal. */
const you: Voter = { telegram_user_id: "4242", username: "dave" };
const peer: Voter = { telegram_user_id: "13579", username: "ada" };
/** A Telegram account need not have a username; identity is the numeric id. */
const nameless: Voter = { telegram_user_id: "9317", username: null };

/** A line the room says somebody got. */
const got = (index: number, by: Voter) => ({ index, by, pantry: null });
/** A line the room says the plan's kitchen already had (#156). */
const jar = (index: number, pantry: string) => ({ index, by: null, pantry });

describe("sharedTicks", () => {
  it("attributes this device's own tap to the person who made it", () => {
    // The room has said nothing about line 2 yet — this is the first paint after the
    // tap, and it is already `dave`'s. Before #210 it was NOBODY here and `dave` one
    // round trip later, which is the flash.
    expect(sharedTicks([], { 2: true }, you)).toEqual({
      2: { by: you, pantry: null },
    });
  });

  it("lets the announcement confirm the tap without restyling it", () => {
    // The same line, once the room's whole-list frame has landed: the tick moves from
    // `inFlight` into `checks` and the screen must not move at all. Asserted as an
    // equality between the two states rather than two separate literals, because
    // "identical" is the property — a difference of any kind is a repaint.
    const optimistic = sharedTicks([], { 2: true }, you);
    const announced = sharedTicks([got(2, you)], {}, you);
    expect(announced).toEqual(optimistic);
  });

  it("takes the colour back with the tick when the room refuses it", () => {
    // A tick the server's own predicate refused still gets a whole-list frame back —
    // the list as it actually is, which does not hold line 2 — and that frame clears
    // `inFlight`. Nothing of the optimism survives it: not the tick, and so not the
    // colour, because they were one entry.
    expect(sharedTicks([], { 2: true }, you)).toHaveProperty("2");
    const rolledBack = sharedTicks([], {}, you);
    expect(rolledBack).toEqual({});
    expect(Object.hasOwn(rolledBack, "2")).toBe(false);
  });

  it("never colours a line the room has already given somebody else", () => {
    // The tap and a peer's confirmed tick racing on one line. The room's word wins on
    // my screen too — an optimistic tick fills a gap, it does not overwrite an answer.
    expect(sharedTicks([got(2, peer)], { 2: true }, you)).toEqual({
      2: { by: peer, pantry: null },
    });
  });

  it("never colours a pantry pre-tick", () => {
    // #156, and #131's rule underneath it: a colour means somebody, and a cupboard is
    // not somebody. A pre-ticked line stays the jar's whatever this device has in
    // flight against it, and keeps naming the entry that answered for it.
    expect(sharedTicks([jar(0, "salt")], { 0: true }, you)).toEqual({
      0: { by: null, pantry: "salt" },
    });
  });

  it("un-ticks ahead of the answer, and strands nothing when it does", () => {
    // Putting a line back is unconditional — it is my line, and the room will agree in
    // a moment. Un-ticking a tap that has not been announced yet leaves no entry
    // behind either: the `false` cancels the `true` rather than layering on it.
    expect(sharedTicks([got(0, you)], { 0: false }, you)).toEqual({});
    expect(sharedTicks([], { 0: false }, you)).toEqual({});
  });

  it("leaves a tap unattributed only while the session is unread", () => {
    // `null` is "this page does not know who it is yet", which is true for the first
    // frames and for nothing else. NOBODY is the honest answer to it.
    expect(sharedTicks([], { 2: true }, null)).toEqual({ 2: NOBODY });
  });

  it("passes the room's list through whole", () => {
    // Everything the server said, in one map keyed by line — including a person with
    // no username, who is identified by the numeric id and rendered by it.
    expect(
      sharedTicks([got(0, you), jar(1, "onion"), got(3, nameless)], {}, you),
    ).toEqual({
      0: { by: you, pantry: null },
      1: { by: null, pantry: "onion" },
      3: { by: nameless, pantry: null },
    });
  });
});

describe("localTicks", () => {
  it("makes a tick from this sitting the tapper's", () => {
    // No meal session, so nobody else can have ticked it and no announcement is coming
    // — it is this person's from the first paint and stays that way.
    expect(localTicks({ 0: true }, { 0: true }, you)).toEqual({
      0: { by: you, pantry: null },
    });
  });

  it("leaves a tick restored from storage nobody's", () => {
    // The list outlives the sitting that wrote it, and the browser is shared. Claiming
    // these for whoever is at the screen now would be a colour meaning "somebody,
    // probably", which is exactly what #131 refuses.
    expect(localTicks({ 0: true, 3: true }, {}, you)).toEqual({
      0: NOBODY,
      3: NOBODY,
    });
  });

  it("tells the two apart on one list", () => {
    // The ordinary state a moment after opening a solo list: what was there, and what
    // has just been got.
    expect(localTicks({ 0: true, 3: true }, { 3: true }, you)).toEqual({
      0: NOBODY,
      3: { by: you, pantry: null },
    });
  });

  it("attributes nothing while the session is unread", () => {
    expect(localTicks({ 0: true }, { 0: true }, null)).toEqual({ 0: NOBODY });
  });

  it("ticks only what the list holds", () => {
    // `mine` is a claim about the list, never an addition to it: a line that is not
    // ticked has no row to colour, so the colour cannot outlive the tick here either.
    expect(localTicks({}, { 0: true }, you)).toEqual({});
  });
});
