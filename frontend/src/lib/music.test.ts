import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CONVERGE_S,
  IN_SYNC_S,
  MAX_NUDGE,
  POOLS,
  SEEK_S,
  elapsedSeconds,
  planChannel,
  planSection,
  playingAt,
  poolFor,
  reconcile,
  runningOrder,
  tuning,
  type Playing,
  type Track,
} from "./music";

/**
 * The plan's radio stations (#212) and a device keeping itself tuned in (#214).
 *
 * The soundtrack is a **pure function** of the plan's seed and how long the plan has
 * existed, so every property that used to need a protocol is now an arithmetic fact that
 * can be checked here — and has to be, because every bug this can have looks like a
 * working feature on the device you are holding. A sign error in the elapsed arithmetic
 * puts a phone whose clock is fast further down the station than everybody else; a
 * shuffle that is not deterministic gives each device its own order while looking
 * perfectly shuffled; an `elapsed % total` that indexes off the end silently drops a
 * plan's music after one cycle. All of those pass a screenshot, a `svelte-check` and a
 * live poke on one laptop.
 */

/** A plan's seed. Any number would do; this one is the fixture the goldens below are
 * stated against. */
const SEED = 4242;

/**
 * **The order seed 4242 deals `buy`** — a golden, and the point of it is that it never
 * changes.
 *
 * Not an oracle for the shuffle's *quality* (the properties below are that); an oracle
 * for its **determinism**. Two devices agree only because `mix`, `mulberry32` and the
 * Fisher–Yates walk answer identically on every engine for ever, so touching any of them
 * — a different constant, a different loop direction, `Math.random` sneaking back in —
 * has to be a decision somebody makes, not a refactor that silently re-deals every plan
 * in flight onto a station its participants' other devices are not on.
 */
const BUY_ORDER = [
  "/music/buy-4.mp3",
  "/music/buy-1.mp3",
  "/music/buy-2.mp3",
  "/music/buy-3.mp3",
];
/** …and their lengths, so the schedule below is hand-computable: 131.904 + 74.616 +
 * 97.416 + 153.072. */
const BUY_TOTAL = 457.008;

describe("runningOrder", () => {
  it("deals the same order for the same seed, for ever", () => {
    expect(runningOrder(SEED, "buy").map((t) => t.src)).toEqual(BUY_ORDER);
    expect(runningOrder(SEED, "cook").map((t) => t.src)).toEqual([
      "/music/cook-1.mp3",
      "/music/cook-3.mp3",
      "/music/cook-2.mp3",
    ]);
  });

  /**
   * **The no-back-to-back-repeat rule, and it is a property rather than a rule.**
   *
   * A permutation has distinct neighbours by construction, and — because the station
   * plays it end to end and starts again — the wrap from the last track to the first is a
   * pair of neighbours too. Checking the wrap is the half that a shuffle which merely
   * "does not repeat within the list" would fail.
   */
  it("is a permutation of the pool, so nothing repeats back to back — including the seam", () => {
    for (const section of ["buy", "cook", "pick", "joy"] as const) {
      const pool = POOLS[section];
      for (let seed = 0; seed < 60; seed++) {
        const order = runningOrder(seed, section);
        expect(order.length, `${section} @ ${seed}`).toBe(pool.length);
        expect(new Set(order.map((t) => t.src))).toEqual(
          new Set(pool.map((t) => t.src)),
        );
        for (let i = 1; i < order.length; i++) {
          expect(order[i].src, `${section} @ ${seed}`).not.toBe(
            order[i - 1].src,
          );
        }
        // The seam: the last track is followed by the first, next cycle.
        if (order.length > 1) {
          expect(order[order.length - 1].src).not.toBe(order[0].src);
        }
      }
    }
  });

  /** Two sections of one plan are two stations, not the same one playing in two rooms —
   * the section is stirred into the seed, so dropping it plays identical music through
   * the whole meal. */
  it("gives a plan a different order per section", () => {
    const buy = runningOrder(SEED, "buy").map((t) => t.src);
    const cook = runningOrder(SEED, "cook").map((t) => t.src);
    expect(buy).not.toEqual(cook);
    // …and every pool is reachable in more than one order across plans, so a shuffle
    // that quietly always dealt the pool's own order would fail here.
    const orders = new Set(
      Array.from({ length: 40 }, (_, seed) =>
        runningOrder(seed, "buy")
          .map((t) => t.src)
          .join(","),
      ),
    );
    expect(orders.size).toBeGreaterThan(1);
  });

  /**
   * **The whole seed is used, not just its low 32 bits.**
   *
   * A seed is 53 bits precisely so it survives JSON into a browser's `Number` exactly,
   * and folding it into the PRNG's 32-bit state has to *combine* its halves rather than
   * truncate — otherwise the seed space is really 2^32 and two plans that differ only
   * above that boundary are dealt the same station while looking perfectly random.
   *
   * Eight seeds identical below 2^32: a shuffle that reads the whole number puts them on
   * more than one order (`buy` has 24), and one that truncates puts all eight on exactly
   * one. Stated as a property rather than a golden pair, so it cannot pass by luck.
   */
  it("uses the whole seed, not just its low 32 bits", () => {
    const orders = new Set(
      Array.from({ length: 8 }, (_, k) =>
        runningOrder(12345 + k * 0x100000000, "buy")
          .map((t) => t.src)
          .join(","),
      ),
    );
    expect(orders.size).toBeGreaterThan(1);
  });

  /** A lone track loops: the relaxation, from the same construction rather than a
   * special case. */
  it("deals a single-track section that one track", () => {
    expect(runningOrder(SEED, "pick").map((t) => t.src)).toEqual(["/pick.mp3"]);
    expect(runningOrder(SEED, "joy")).toEqual([]);
  });
});

describe("playingAt", () => {
  const at = (elapsed: number) => playingAt(SEED, "buy", elapsed);

  /**
   * **The hand-computed schedule.** Seed 4242's `buy` order is
   * `buy-4 (131.904) · buy-1 (74.616) · buy-2 (97.416) · buy-3 (153.072)`, so at 250
   * seconds into the plan the needle is `250 − 131.904 − 74.616 = 43.48` seconds into
   * `buy-2` — arithmetic done here on paper, not by asking the implementation twice.
   */
  it("walks the running order by the tracks' own lengths", () => {
    expect(at(0)?.track.src).toBe("/music/buy-4.mp3");
    expect(at(0)?.position).toBe(0);

    expect(at(100)?.track.src).toBe("/music/buy-4.mp3");
    expect(at(100)?.position).toBeCloseTo(100, 6);

    // Exactly on a boundary belongs to the track that is starting, not the one ending.
    expect(at(131.904)?.track.src).toBe("/music/buy-1.mp3");
    expect(at(131.904)?.position).toBeCloseTo(0, 6);

    expect(at(200)?.track.src).toBe("/music/buy-1.mp3");
    expect(at(200)?.position).toBeCloseTo(200 - 131.904, 6);

    expect(at(250)?.track.src).toBe("/music/buy-2.mp3");
    expect(at(250)?.position).toBeCloseTo(43.48, 6);

    expect(at(400)?.track.src).toBe("/music/buy-3.mp3");
    expect(at(400)?.position).toBeCloseTo(400 - 131.904 - 74.616 - 97.416, 6);
  });

  /**
   * **The cycle wraps**, which is what makes a plan that runs for days a modulo rather
   * than a week of simulated rollovers. Drop the `% total` and a station goes silent —
   * or sticks on its last track — the moment the first cycle ends.
   */
  it("starts the order again when it reaches the end", () => {
    expect(at(BUY_TOTAL)?.track.src).toBe("/music/buy-4.mp3");
    expect(at(BUY_TOTAL)?.position).toBeCloseTo(0, 6);

    // A plan running for days answers the same as one running for minutes. The elapsed
    // values here are deliberately *not* on a track boundary: `BUY_TOTAL * 100` is not
    // exactly representable, so a value within a float epsilon of a seam can land at the
    // end of the track before it rather than the start of the one after — a difference of
    // under a microsecond of real time, which the reconcile loop closes on its next check
    // and which no listener could be on the wrong side of for long enough to hear.
    for (const elapsed of [100, 250, 400]) {
      const once = at(elapsed);
      for (const cycles of [1, 2, 100]) {
        const later = at(elapsed + BUY_TOTAL * cycles);
        expect(later?.track.src, `${elapsed} + ${cycles} cycles`).toBe(
          once?.track.src,
        );
        expect(later?.position).toBeCloseTo(once!.position, 4);
      }
    }
  });

  /** A plan whose stored birth second is a moment ahead of this device's corrected
   * clock wraps in from the other end rather than indexing off the front. */
  it("wraps a negative elapsed into the cycle", () => {
    const back = at(-1);
    expect(back?.track.src).toBe("/music/buy-3.mp3");
    expect(back?.position).toBeCloseTo(153.072 - 1, 6);
    expect(at(-BUY_TOTAL - 1)?.track.src).toBe("/music/buy-3.mp3");
  });

  /** A section with no tracks has no station — silence, honestly, rather than a chosen
   * silence or another section's music. */
  it("has nothing to play for a section with no tracks", () => {
    expect(playingAt(SEED, "joy", 0)).toBeNull();
    expect(playingAt(SEED, "joy", 99_999)).toBeNull();
  });

  /** A single-track section is the same walk with one entry: it just loops. */
  it("loops a single-track section", () => {
    expect(playingAt(SEED, "pick", 0)?.position).toBeCloseTo(0, 6);
    expect(playingAt(SEED, "pick", 119.664)?.position).toBeCloseTo(0, 6);
    expect(playingAt(SEED, "pick", 150)?.track.src).toBe("/pick.mp3");
    expect(playingAt(SEED, "pick", 150)?.position).toBeCloseTo(
      150 - 119.664,
      6,
    );
  });
});

describe("elapsedSeconds", () => {
  /**
   * **The hand-computed fixture.** A plan born at unix second 1_700_000_000; this
   * device's clock has been measured 5,000ms fast and currently reads
   * 1_700_000_042_000. Its own reading is 42 seconds past the plan's birth, but 5 of
   * those are its clock being wrong, so the plan is **37 seconds** old — and that is
   * where this device must play, not 42.
   */
  it("takes this device's measured drift off its own clock", () => {
    expect(elapsedSeconds(1_700_000_000, 5_000, 1_700_000_042_000)).toBe(37);
  });

  /** The same instant on a phone a minute *slow*: it reads a smaller `now`, so its
   * offset is negative and comes off as an addition. Both answer the same age, which is
   * the whole reason the clock is measured at all. */
  it("agrees between two phones whose clocks disagree", () => {
    const fast = elapsedSeconds(1_700_000_000, 60_000, 1_700_000_090_000);
    const slow = elapsedSeconds(1_700_000_000, -60_000, 1_699_999_970_000);
    expect(fast).toBe(30);
    expect(slow).toBe(30);
  });

  it("is zero at the instant the plan is born", () => {
    expect(elapsedSeconds(1_700_000_000, 0, 1_700_000_000_000)).toBe(0);
  });
});

describe("tuning", () => {
  /**
   * **A plan with no seed is a device on its own** — the whole degrade story, and the
   * one that has to be a test rather than a comment. Plans created before migration 0031
   * carry `null`; inventing a seed for them now would invent a shared past those
   * participants did not have, and defaulting to `0` would put every one of them on the
   * same station.
   */
  it("leaves a plan with no seed to the lone device's pool", () => {
    expect(tuning("ab12", "buy", null, 100)).toEqual({ kind: "alone" });
  });

  /** …and so is a page with no plan at all. */
  it("leaves a page with no plan alone", () => {
    expect(tuning(null, "buy", SEED, 100)).toEqual({ kind: "alone" });
    expect(tuning("ab12", null, SEED, 100)).toEqual({ kind: "alone" });
  });

  /** Not yet told is **not** the same as "has none": it lasts one round trip, and
   * starting a private track across it only to abandon it is worse than a moment of
   * quiet. Collapsing this into `alone` is a real desync — every device would open on
   * its own random track before joining the station. */
  it("waits when the plan's seed has not arrived yet", () => {
    expect(tuning("ab12", "buy", undefined, 100)).toEqual({ kind: "waiting" });
  });

  it("tunes in when there is a plan, a section and a seed", () => {
    const tuned = tuning("ab12", "buy", SEED, 250);
    expect(tuned.kind).toBe("station");
    expect(tuned.kind === "station" && tuned.playing?.track.src).toBe(
      "/music/buy-2.mp3",
    );
  });

  /** A station whose section has no tracks is still a station: silence, not the lone
   * device's pool — otherwise `joy` in a plan would play something nobody else hears. */
  it("tunes into a silent station rather than falling back", () => {
    expect(tuning("ab12", "joy", SEED, 100)).toEqual({
      kind: "station",
      playing: null,
    });
  });
});

describe("planChannel", () => {
  it("reads the plan out of the pick's own URL", () => {
    expect(planChannel("/pick/ab12cd34", undefined)).toBe("ab12cd34");
    // Percent-encoded on the way in (`PickClient.url`), so read back the same way.
    expect(planChannel("/pick/a%2Fb", undefined)).toBe("a/b");
  });

  it("carries the plan into buy, cook and joy with the stashed decision", () => {
    for (const path of ["/buy", "/cook", "/joy"]) {
      expect(planChannel(path, "ab12cd34")).toBe("ab12cd34");
    }
  });

  /** Half the solo story is this null (the other half is a plan with no seed). Every one
   * of these is a page that must never open a room, and a widening of this function is
   * how solo traffic would appear. */
  it("has no room to open where there is no plan", () => {
    expect(planChannel("/kitchens", "ab12cd34")).toBeNull();
    expect(planChannel("/kitchens/k1/pantry", "ab12cd34")).toBeNull();
    expect(planChannel("/", "ab12cd34")).toBeNull();
    expect(planChannel("/health", "ab12cd34")).toBeNull();
    expect(planChannel("/pick", "ab12cd34")).toBeNull();
    expect(planChannel("/buy", undefined)).toBeNull();
    expect(planChannel("/cook", null)).toBeNull();
  });
});

describe("planSection", () => {
  it("names the four legs of a meal, and nothing else", () => {
    expect(planSection("/pick/ab12")).toBe("pick");
    expect(planSection("/buy")).toBe("buy");
    expect(planSection("/cook")).toBe("cook");
    expect(planSection("/joy")).toBe("joy");
    // Kitchens has music and is not something a room does together.
    expect(planSection("/kitchens")).toBeNull();
    expect(planSection("/")).toBeNull();
  });
});

describe("POOLS", () => {
  /**
   * **Every declared length is the file's own length**, counted frame by frame.
   *
   * A station's schedule is these numbers laid end to end, so a declared length that is
   * not the file's slides every device's idea of the running order against the audio it
   * is actually playing — identically on every device, which is the mercy, but wrong for
   * all of them and invisible until somebody listens for two minutes. Replacing an asset
   * with one of a different length has to fail here.
   *
   * MPEG-1/2 Layer III frames carry their own bitrate and sample rate, and a frame is a
   * fixed number of samples, so walking the file and summing `samples / rate` is the
   * length — exact for constant and variable bitrate alike, and no dependency.
   */
  it("declares, for every track, the length the file actually is", () => {
    const V1 = [
      0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const V2 = [
      0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];
    const RATES: Record<number, number[]> = {
      3: [44100, 48000, 32000],
      2: [22050, 24000, 16000],
      0: [11025, 12000, 8000],
    };
    const lengthOf = (buf: Buffer): number => {
      let i = 0;
      if (buf.subarray(0, 3).toString("latin1") === "ID3") {
        i =
          10 +
          (((buf[6] & 0x7f) << 21) |
            ((buf[7] & 0x7f) << 14) |
            ((buf[8] & 0x7f) << 7) |
            (buf[9] & 0x7f));
      }
      let total = 0;
      while (i + 4 <= buf.length) {
        if (buf[i] !== 0xff || (buf[i + 1] & 0xe0) !== 0xe0) {
          i++;
          continue;
        }
        const version = (buf[i + 1] >> 3) & 0x03;
        const layer = (buf[i + 1] >> 1) & 0x03;
        const bitrateIdx = (buf[i + 2] >> 4) & 0x0f;
        const rateIdx = (buf[i + 2] >> 2) & 0x03;
        const padding = (buf[i + 2] >> 1) & 0x01;
        if (
          version === 1 ||
          layer !== 1 ||
          bitrateIdx === 0 ||
          bitrateIdx === 15 ||
          rateIdx === 3
        ) {
          i++;
          continue;
        }
        const mpeg1 = version === 3;
        const bitrate = (mpeg1 ? V1 : V2)[bitrateIdx] * 1000;
        const rate = RATES[version][rateIdx];
        const samples = mpeg1 ? 1152 : 576;
        total += samples / rate;
        i += Math.floor((samples / 8) * (bitrate / rate)) + padding;
      }
      return total;
    };

    const root = new URL("../../static", import.meta.url).pathname;
    const declared = new Map<string, number>();
    for (const pool of Object.values(POOLS)) {
      for (const track of pool) declared.set(track.src, track.seconds);
    }
    expect(declared.size).toBeGreaterThan(0);
    for (const [src, seconds] of declared) {
      const measured = lengthOf(readFileSync(join(root, src)));
      // One frame is ~26ms; anything larger is a different file, not rounding.
      expect(measured, `${src}`).toBeCloseTo(seconds, 2);
    }
  });

  /** And every file a pool names exists at all — the failure that would otherwise be a
   * room told to play a 404, which is silence that looks exactly like the bug all this
   * fixes. */
  it("names only files that exist under static/", () => {
    const root = new URL("../../static", import.meta.url).pathname;
    const present = new Set<string>();
    const walk = (dir: string, prefix: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.isDirectory()) {
          walk(join(dir, entry.name), `${prefix}${entry.name}/`);
        } else present.add(`/${prefix}${entry.name}`);
      }
    };
    walk(root, "");
    for (const [route, pool] of Object.entries(POOLS)) {
      for (const track of pool) {
        expect(present.has(track.src), `${route} names ${track.src}`).toBe(
          true,
        );
      }
    }
  });

  it("serves a route with no music an empty pool rather than someone else's", () => {
    expect(poolFor("/health")).toEqual([]);
    expect(poolFor("/joy")).toEqual([]);
    expect(poolFor("/cook").length).toBeGreaterThan(1);
  });
});

describe("reconcile", () => {
  const track: Track = { src: "/music/buy-2.mp3", seconds: 97.416 };
  /** The station, `position` seconds into that track. */
  const want = (position: number): Playing => ({ track, position });
  /** This device, `at` seconds into `src`. */
  const actual = (at: number, src: string | null = track.src) => ({
    track: src,
    currentTime: at,
  });

  /** The deadband, at its edge — measured from zero so the numbers are exact rather than
   * an artefact of adding two decimals. `IN_SYNC_S` itself still holds: the correction
   * stops *at* the threshold, which is what keeps it from hunting around zero. */
  it("holds when the difference is inaudible", () => {
    expect(reconcile(want(0), actual(0))).toEqual({ kind: "hold" });
    expect(reconcile(want(0), actual(IN_SYNC_S))).toEqual({ kind: "hold" });
    expect(reconcile(want(0), actual(-IN_SYNC_S))).toEqual({ kind: "hold" });
    expect(reconcile(want(0), actual(IN_SYNC_S * 2)).kind).toBe("nudge");
  });

  /**
   * **Ahead of the station ⇒ slow down.** The sign is the whole branch: run it the other
   * way and every device races further away the harder it tries to catch up, which is a
   * bug that sounds like nothing at all on the device you are testing on.
   */
  it("nudges slower when this device is ahead, and faster when behind", () => {
    const ahead = reconcile(want(30), actual(30.2));
    const behind = reconcile(want(30), actual(29.8));
    expect(ahead.kind).toBe("nudge");
    expect(behind.kind).toBe("nudge");
    // Proportional, so it eases off as it converges instead of overshooting and hunting
    // back — 0.2s out of a 10s convergence is 2% under normal speed.
    expect(ahead.kind === "nudge" && ahead.rate).toBeCloseTo(
      1 - 0.2 / CONVERGE_S,
      6,
    );
    expect(behind.kind === "nudge" && behind.rate).toBeCloseTo(
      1 + 0.2 / CONVERGE_S,
      6,
    );
    expect(ahead.kind === "nudge" && ahead.rate).toBeLessThan(1);
    expect(behind.kind === "nudge" && behind.rate).toBeGreaterThan(1);
  });

  /** The nudge is capped where a listener would start to hear it as a change of pitch
   * rather than a change of when. */
  it("never nudges past the inaudible cap", () => {
    expect(reconcile(want(30), actual(30 + SEEK_S - 0.001))).toEqual({
      kind: "nudge",
      rate: 1 - MAX_NUDGE,
    });
  });

  /**
   * **The nudge/seek split**, exactly at the threshold. Below it the repair is
   * inaudible; at it and above, the difference is a stall or a wake rather than drift,
   * and a nudge would leave two phones in one kitchen audibly out of phase for the ~17
   * checks it would take to close.
   */
  it("seeks at the threshold and nudges just under it", () => {
    expect(reconcile(want(0), actual(SEEK_S))).toEqual({
      kind: "seek",
      position: 0,
    });
    expect(reconcile(want(0), actual(-SEEK_S))).toEqual({
      kind: "seek",
      position: 0,
    });
    expect(reconcile(want(0), actual(SEEK_S - 0.0001)).kind).toBe("nudge");
  });

  /** A phone that slept and woke a whole track later is a seek, not a slow crawl. */
  it("seeks a device that was asleep back into the station", () => {
    expect(reconcile(want(90), actual(12))).toEqual({
      kind: "seek",
      position: 90,
    });
  });

  /**
   * **On the wrong track is not drift.** The station has moved on — a rollover while
   * this device was asleep, or it has just arrived — so load the current one at the
   * current position. Ranked first because no position on the wrong song means anything.
   */
  it("loads the station's track when this device is on another one", () => {
    expect(reconcile(want(12), actual(90, "/music/buy-1.mp3"))).toEqual({
      kind: "load",
      track,
      position: 12,
    });
    // Including from silence, which is what a device that has just arrived is.
    expect(reconcile(want(12), actual(0, null)).kind).toBe("load");
  });

  /** No branch ever asks an element for a place no track has. */
  it("never asks for a negative position", () => {
    expect(reconcile(want(-0.4), actual(3))).toEqual({
      kind: "seek",
      position: 0,
    });
    expect(reconcile(want(-0.4), actual(0, null))).toEqual({
      kind: "load",
      track,
      position: 0,
    });
  });
});
