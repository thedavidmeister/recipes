import type { Section } from "./types";

/**
 * **The music, as a room hears it** (#212) and **as a device keeps itself in it** (#214).
 *
 * The soundtrack used to be a per-device dice roll: every browser picked its own random
 * track from the section's pool and started it whenever that device happened to arrive.
 * Two people shopping the same list heard different songs at different points, in
 * something presented as the *meal's* atmosphere.
 *
 * ## The soundtrack is a function, not state
 *
 * > "why don't we just have a seed for the meal plan that we can dangle all randomness
 * > off that everyone can share, and the soundtrack just starts at the meal creation
 * > timestamp?" — ruled
 *
 * **Each section is a radio station that has been playing since the plan was born.** Its
 * running order is a shuffle of the section's pool keyed on the plan's seed, and where
 * the needle is depends only on how long the plan has existed. So a device does not ask
 * what is playing and is never *told* — it computes:
 *
 * ```text
 * (plan seed, section)            → the running order          runningOrder()
 * (now − this device's drift − the plan's birth instant)
 *                                 → elapsed                    elapsedSeconds()
 * (running order, elapsed)        → this track, this position   playingAt()
 * ```
 *
 * Everything that made the alternative hard stops existing rather than being solved.
 * There is no music table, no music event, no authority over what plays next, no race
 * when several devices notice a track end at the same moment, no frame to miss and no
 * state to rehydrate. A phone that has been asleep for an hour computes the same answer
 * as one that has been watching the whole time, because both are computing rather than
 * remembering — which is also why "a missed rollover" is not a concept here: a rollover
 * is just this function's value changing.
 *
 * Two things are still deliberately not derived:
 *
 * - **Whether a device makes a sound is personal.** The on/off switch is untouched. Sync
 *   decides *what* plays and *where in it we are*, never whether your phone is audible —
 *   and there is nothing to guard, because listening is reading two numbers and a clock
 *   (#200: watchers hear the room).
 * - **A plan with no seed has no shared soundtrack.** Plans created before migration
 *   0031 carry `null`, honestly, and get the lone device's random pick — the same path a
 *   page with no plan behind it takes.
 *
 * ## Pure, and its own module
 *
 * Nothing here fetches, reads `$env`, touches an element or reads a clock — the
 * `$lib/shopping` split (#176/7, enforced by `lint:env`), so the unit runner reaches all
 * of it. The layout owns the two `<audio>` voices, the fade and the socket, and hands
 * this module numbers.
 */

/**
 * One track, and **how long it is**.
 *
 * The length is not decoration: a station's schedule is the running order laid end to
 * end, so where the needle sits at a given elapsed time is arithmetic over these numbers.
 * A device could read the length off its own `<audio>` element instead, but only for the
 * track it has already loaded — which is exactly the track it is trying to work out.
 *
 * The values are **measured from the files**, frame by frame, by
 * `every_track_is_the_file_it_says_it_is` — not estimated, and not trusted: replacing an
 * asset with one of a different length fails that test rather than quietly sliding every
 * device's schedule against the audio.
 */
export interface Track {
  src: string;
  seconds: number;
}

/**
 * **The tracks each route can play.**
 *
 * One list, used two ways: a plan's station shuffles it deterministically from the plan's
 * seed, and a lone device picks from it at random. There is no second copy anywhere —
 * the server has no idea what a track is, because with the soundtrack a function of the
 * seed there is nothing for it to decide.
 *
 * `kitchens` is here and is not a section a plan has: standing in your own kitchen is not
 * something a room does together, so it has no station and never will.
 *
 * **`joy` is empty on purpose** — its own tracks are still to come, and an empty pool is
 * silence, which is the honest state for a section whose music does not exist yet rather
 * than borrowing another section's (#192's rule about a deck that must never contain a
 * guess).
 *
 * **Changing a pool moves every device identically**, and that is the accepted cost of
 * deriving rather than storing: adding or replacing a track changes what the function
 * answers, so at the deploy every listening device jumps to the new schedule *together*.
 * One synchronised change, and the room stays together across it — which is the property
 * that matters. Nothing drifts apart, and nobody is left on a track the others are not.
 */
export const POOLS: Record<string, Track[]> = {
  kitchens: [
    { src: "/music/title-1.mp3", seconds: 71.352 },
    { src: "/music/title-2.mp3", seconds: 170.976 },
    { src: "/music/title-3.mp3", seconds: 101.568 },
    { src: "/music/title-4.mp3", seconds: 93.648 },
  ],
  pick: [{ src: "/pick.mp3", seconds: 119.664 }],
  buy: [
    { src: "/music/buy-1.mp3", seconds: 74.616 },
    { src: "/music/buy-2.mp3", seconds: 97.416 },
    { src: "/music/buy-3.mp3", seconds: 153.072 },
    { src: "/music/buy-4.mp3", seconds: 131.904 },
  ],
  cook: [
    { src: "/music/cook-1.mp3", seconds: 131.952 },
    { src: "/music/cook-2.mp3", seconds: 109.344 },
    { src: "/music/cook-3.mp3", seconds: 179.976 },
  ],
  joy: [],
};

const NONE: Track[] = [];

/** The pool for a route, or nothing where a route has no music. */
export function poolFor(pathname: string): Track[] {
  return POOLS[pathname.split("/")[1]] ?? NONE;
}

/**
 * A random track from `pool`, avoiding `exclude` so a pool of several does not repeat
 * back to back — **the lone device's shuffle**, and the only thing here that is random at
 * the moment it is called.
 *
 * The rule relaxes when there is nothing else to honour it with (a single-track pool, or
 * one whose remaining tracks are all `exclude`) and returns what there is, so a lone
 * track just keeps playing. A room's equivalent is {@link runningOrder}, which gets the
 * same property from being a permutation rather than from a rule.
 */
export function pickFrom(pool: Track[], exclude: string | null): Track {
  const others = pool.filter((track) => track.src !== exclude);
  const options = others.length ? others : pool;
  return options[Math.floor(Math.random() * options.length)];
}

/** The four legs of a meal — the sections a plan has a station for. */
const SECTIONS: Section[] = ["pick", "buy", "cook", "joy"];

/**
 * Which leg of the meal this page is on, or `null` for a page that is not part of one.
 *
 * `kitchens` and the home page have music and are not a meal's sections, so they answer
 * `null` and are played the lone device's way even for somebody who is in a plan — there
 * is no room standing in a kitchen.
 */
export function planSection(pathname: string): Section | null {
  const first = pathname.split("/")[1];
  return SECTIONS.find((s) => s === first) ?? null;
}

/**
 * **The plan whose room this page belongs to**, or `null` when there is none.
 *
 * The layout renders above every page and holds the player, but it is the *pages* that
 * know which plan is underway, and they know it in the two different ways the app already
 * stores it. This reads both rather than inventing a third:
 *
 * - **`/pick/<channel>`** — the plan is in the URL, which is what makes the link
 *   shareable in the first place.
 * - **`buy`, `cook`, `joy`** — the plan travels with the decision the pick stashed
 *   (`$lib/buy`'s `consensusRef`), which is exactly where `getBuyList` and
 *   `getCookRecipe` read it from, so the layout and the page underneath it can never
 *   disagree about which room they are in.
 *
 * `consensus` is passed in rather than read here so this module stays pure and reachable
 * from the unit runner (`$lib/buy` reaches `$env` through `$lib/client`; `lint:env`).
 *
 * **`null` is half the solo story** — the other half is a plan with no seed. Either way:
 * no socket, no station, no reconcile loop, and the lone device's random pick untouched.
 */
export function planChannel(
  pathname: string,
  consensus: string | null | undefined,
): string | null {
  const parts = pathname.split("/");
  if (parts[1] === "pick") {
    return parts[2] ? decodeURIComponent(parts[2]) : null;
  }
  if (planSection(pathname)) return consensus ?? null;
  return null;
}

// ---- the station ------------------------------------------------------------

/**
 * **How long the plan has existed**, in seconds, as this device should reckon it.
 *
 * The one piece of arithmetic the whole feature rests on. `createdAt` is a shared
 * instant (the plan's `created_at`, unix **seconds**) and `Date.now()` is this device's
 * clock, so the two are only comparable once the device's own measured drift is taken out
 * of the second one. Spelled out rather than delegated, because reading it in place is
 * what makes the direction checkable: a phone five minutes fast reads a *larger* `now`,
 * so its offset must come **off** it, or that phone would think the plan is five minutes
 * older than it is and play five minutes further down the station.
 *
 * Whole seconds on the anchor are enough because **every device reads the same stored
 * value**: the coarseness is shared, so it moves the whole room by the same fraction of a
 * second and disagrees with nobody.
 */
export function elapsedSeconds(
  createdAt: number,
  offsetMs: number,
  nowLocal: number,
): number {
  return (nowLocal - offsetMs) / 1000 - createdAt;
}

/**
 * A 32-bit state for {@link mulberry32}, from a plan's seed and a section's name.
 *
 * The section is stirred in so the four stations of one plan are four different
 * sequences rather than the same one played in four rooms. FNV-1a's prime over the
 * section's characters is enough mixing for a list of at most four tracks, and being
 * plain integer arithmetic it is identical on every engine — which is the only property
 * that actually matters here, since two devices disagreeing about this disagree about
 * everything downstream.
 *
 * The seed is folded from 53 bits to 32 by combining its halves rather than truncating,
 * so two plans differing only in their high bits are still different stations.
 */
function mix(seed: number, section: string): number {
  let h = (Math.floor(seed / 0x100000000) ^ (seed >>> 0)) >>> 0;
  for (let i = 0; i < section.length; i++) {
    h = Math.imul(h ^ section.charCodeAt(i), 0x01000193) >>> 0;
  }
  return h;
}

/** mulberry32 — a small, fast, fully specified PRNG. Deliberately not `Math.random`:
 * this has to give the same sequence on every device, for ever. */
function mulberry32(state: number): () => number {
  let a = state >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), 1 | t);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 0x100000000;
  };
}

/**
 * **The station's running order**: this section's pool, shuffled by this plan's seed.
 *
 * A Fisher–Yates shuffle driven by a seeded PRNG, so it is a *permutation* — every track
 * once — and the station plays it end to end and starts again.
 *
 * That shape is where the no-back-to-back-repeat rule comes from, and it comes for free
 * rather than being enforced: the tracks in a permutation are distinct, so no two
 * neighbours are the same, **and the wrap from the last track to the first is a pair of
 * distinct tracks too** for any pool of two or more. A pool of one loops, which is the
 * same relaxation {@link pickFrom} makes for the same reason — there is nothing else to
 * play.
 *
 * One shuffle for the room, and it is one shuffle *per plan*: two plans running at the
 * same moment hear different orders, and the same plan reopened tomorrow hears the order
 * it has always had.
 */
export function runningOrder(seed: number, section: Section): Track[] {
  const order = [...(POOLS[section] ?? NONE)];
  const random = mulberry32(mix(seed, section));
  for (let i = order.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [order[i], order[j]] = [order[j], order[i]];
  }
  return order;
}

/** What a station is playing, and how far into it. */
export interface Playing {
  track: Track;
  /** Seconds into {@link track}. */
  position: number;
}

/**
 * **Where the needle is**: the track this section is playing `elapsed` seconds into the
 * plan's life, and how far into it.
 *
 * The running order laid end to end is a cycle `total` seconds long, so the answer is
 * `elapsed mod total` walked against the order. Constant work however old the plan is —
 * a plan running for a week is one modulo, not a week of simulated rollovers — which is
 * the property that makes deriving cheaper than remembering rather than merely tidier.
 *
 * `null` when the section has no tracks (`joy`, today): no station, and silence rather
 * than a chosen silence.
 *
 * A negative `elapsed` — a plan whose stored birth second is a moment ahead of this
 * device's corrected clock — wraps into the cycle from the other end rather than
 * indexing off it, and a second later it is ordinary.
 *
 * Track lengths are decimals, so a value within a float epsilon of a track boundary can
 * answer "the end of the track before" rather than "the start of the track after". That
 * is under a microsecond of real time and it is the same answer on every device, so it
 * is a rounding fact rather than a desync — and the reconcile loop closes it on its next
 * check regardless.
 */
export function playingAt(
  seed: number,
  section: Section,
  elapsed: number,
): Playing | null {
  const order = runningOrder(seed, section);
  if (order.length === 0) return null;
  const total = order.reduce((sum, t) => sum + t.seconds, 0);
  let t = elapsed % total;
  if (t < 0) t += total;
  for (let i = 0; i < order.length; i++) {
    // The last track takes whatever is left, so accumulated floating-point error at the
    // very end of a cycle answers "the last track, at its end" rather than falling out
    // of the loop with no answer at all.
    if (t < order[i].seconds || i === order.length - 1) {
      return { track: order[i], position: Math.min(t, order[i].seconds) };
    }
    t -= order[i].seconds;
  }
  /* c8 ignore next */
  return null;
}

/**
 * **What this device should be listening to** — the one branch the whole feature turns
 * on, as a function rather than as a condition spread across the player.
 *
 * Three answers, and they are genuinely three:
 *
 * - **`waiting`** — there is a plan, and this device has not been told its seed yet.
 *   That lasts one round trip after connecting, and the honest thing to do with it is
 *   nothing: starting a private track now only to abandon it a moment later is worse
 *   than a moment of quiet.
 * - **`alone`** — no plan, or a plan created before plans had a seed (migration 0031).
 *   Either way there is no shared randomness, so the lone device's random pick is
 *   correct rather than a fallback: a plan with no seed had no shared past, and
 *   inventing one now would be inventing data (#146 degrades, it does not die).
 * - **`station`** — tune in. `playing` is `null` when the section has no tracks (`joy`,
 *   today), which is silence honestly rather than a chosen silence.
 */
export type Tuning =
  | { kind: "waiting" }
  | { kind: "alone" }
  | { kind: "station"; playing: Playing | null };

export function tuning(
  channel: string | null,
  section: Section | null,
  seed: number | null | undefined,
  elapsed: number,
): Tuning {
  if (channel === null || section === null) return { kind: "alone" };
  if (seed === undefined) return { kind: "waiting" };
  if (seed === null) return { kind: "alone" };
  return { kind: "station", playing: playingAt(seed, section, elapsed) };
}

// ---- self-healing (#214) ----------------------------------------------------

/**
 * How often a device checks itself against the station, in ms.
 *
 * Playback is physical — a buffering stall, a throttled background tab, a slow decode, a
 * phone that slept — so desync is a condition to notice and repair rather than a state to
 * settle into. A second is well inside the {@link SEEK_S} threshold at any plausible rate
 * of drift, so nothing audible has time to accumulate between two checks, and it is a
 * rate a sleeping tab costs nothing for.
 */
export const RECONCILE_MS = 1000;

/**
 * Close enough, in seconds: at or under this the device is in sync and plays at 1×.
 *
 * 50ms is below what a listener can hear as a phase difference between two speakers in
 * one room, and it is comfortably above the jitter of reading `currentTime` off a media
 * element — so a deadband here is what stops the correction hunting around zero forever.
 */
export const IN_SYNC_S = 0.05;

/**
 * Where a nudge stops being enough and the fix is a **hard seek**, in seconds.
 *
 * The split is the shape #214 asks for: small drift heals inaudibly, large drift is
 * repaired at once. Half a second is drift a nudge would take ~17 checks to close, and by
 * then two phones in one kitchen have been audibly out of phase for that whole time —
 * which is worse than one clean jump. Anything above this is not drift anyway: it is a
 * stall, a wake from sleep, or a tab that was throttled, and none of those converge.
 */
export const SEEK_S = 0.5;

/**
 * The fastest a nudge may run, as a fraction of normal speed.
 *
 * 3% is inside what a listener hears as a change of pitch or tempo on music — it is the
 * range a DJ's pitch fader lives in for exactly that reason — so a correction at this
 * rate is a change of *when*, not of *what*. Faster would close the gap sooner and would
 * be the click this design exists to avoid.
 */
export const MAX_NUDGE = 0.03;

/**
 * How long a nudge aims to take to close the gap, in seconds.
 *
 * The rate is proportional — `drift / CONVERGE_S`, capped at {@link MAX_NUDGE} — so a
 * large-ish drift runs at the cap and eases off as it converges instead of overshooting
 * and hunting back. Ten seconds means anything under 0.3s of drift is corrected without
 * ever reaching the cap.
 */
export const CONVERGE_S = 10;

/**
 * **What a device should do to be where the station is.** Strictly local convergence:
 * there is no branch here that could reach another device even in principle, because
 * there is nothing shared to write.
 */
export type Repair =
  /** Playing the wrong track, or nothing. The station has moved on — a rollover while
   * this device was asleep, or it has just arrived, or it was silent. Load and start at
   * the station's position; the same act in all three cases, because there is only one
   * truth and it is computed. */
  | { kind: "load"; track: Track; position: number }
  /** Too far out to heal by ear: a stall, or a wake from sleep. Jump. */
  | { kind: "seek"; position: number }
  /** Drifting: run slightly fast or slow until it converges. `rate` is under 1 when this
   * device is *ahead* of the station. */
  | { kind: "nudge"; rate: number }
  /** In sync. Play at 1×. */
  | { kind: "hold" };

function clamp(value: number, limit: number): number {
  return Math.max(-limit, Math.min(limit, value));
}

/**
 * **Compare where this device is against where the station is, and answer with the
 * repair.**
 *
 * Both halves of `want` are computed rather than received, so this runs on a schedule
 * rather than at track start and needs nothing to have arrived: whatever went wrong —
 * a stall, a throttled tab, a phone that slept through two tracks — the next check
 * answers with the whole truth.
 *
 * The branches are in the order they are because each rules the next one's question out:
 * a position on the wrong track means nothing, so that is asked first; then the split
 * between a jump and an inaudible correction; then the deadband.
 *
 * `position` is never negative — {@link playingAt} answers inside the cycle — and is
 * clamped here anyway so no branch can ask an element for a place no track has.
 */
export function reconcile(
  want: Playing,
  actual: { track: string | null; currentTime: number },
): Repair {
  const position = Math.max(0, want.position);
  if (actual.track !== want.track.src) {
    return { kind: "load", track: want.track, position };
  }
  const drift = actual.currentTime - position;
  if (Math.abs(drift) >= SEEK_S) return { kind: "seek", position };
  if (Math.abs(drift) > IN_SYNC_S) {
    // Ahead of the station ⇒ slow down. The sign is the whole of it: run this the other
    // way and every device races further away the harder it tries to catch up.
    return { kind: "nudge", rate: 1 - clamp(drift / CONVERGE_S, MAX_NUDGE) };
  }
  return { kind: "hold" };
}
