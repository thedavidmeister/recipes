import type { Section } from "./types";

/**
 * **The music, as a room hears it** (#212) and **as a device keeps itself in it** (#214).
 *
 * The soundtrack used to be a per-device dice roll: every browser's layout picked its
 * own random track from the section's pool and started it whenever that device happened
 * to arrive. Two people shopping the same list heard different songs at different
 * points, in something presented as the *meal's* atmosphere.
 *
 * In a plan the **session owns the soundtrack**. The room's state is two facts — which
 * track, and the instant it started on the shared timeline — carried as events on the
 * app's time-sync framework (`$lib/session-events`, `backend/src/events.rs`). Everything
 * else is derived here:
 *
 * - **where in the track the room is**, from that instant and this device's own measured
 *   clock offset ({@link expectedPosition});
 * - **what this device should do about the difference** between that and what its
 *   `<audio>` element is actually doing ({@link reconcile}).
 *
 * Two things are deliberately *not* here, because they are not the room's:
 *
 * - **Which track plays is the server's choice.** The room's pool and its
 *   no-back-to-back-repeat rule live in `backend/src/music.rs` — one shuffle for the
 *   room, and no way for one phone to name a URL every other phone would load. {@link POOLS}
 *   below is the **lone device's** pool, for a page with no plan behind it.
 * - **Whether a device makes a sound is personal.** The on/off switch is untouched: sync
 *   decides *what* plays and *where in it we are*, never whether your phone is audible.
 *
 * ## Pure, and its own module
 *
 * Nothing here fetches, reads `$env`, touches an element or reads a clock — the
 * `$lib/shopping` split (#176/7, enforced by `lint:env`), so the unit runner can reach
 * all of it. The layout owns the two `<audio>` voices, the fade and the socket, and
 * hands this module numbers.
 */

/**
 * **One section's soundtrack, as the room holds it.** Mirrors `music::RoomTrack`.
 *
 * `started_at` is a **shared-timeline** instant — translate through this connection's
 * measured offset before comparing it to `Date.now()`, never before (the `RunningTimer`
 * rule).
 */
export interface RoomTrack {
  /** Which leg of the meal this is the soundtrack for. */
  section: Section;
  /** The track everybody in the plan is playing. Chosen server-side. */
  track: string;
  /** When it started, in the shared timeline (unix ms). */
  started_at: number;
}

/**
 * **A lone device's pools** — a track set per top-level route, every route played by the
 * same code (#88, #121, #125).
 *
 * This is the no-plan path and only that. With a plan behind the page the track is the
 * room's and arrives on the socket, so nothing here is consulted: the server holds the
 * room's pools (`backend/src/music.rs`) because the shuffle and the no-repeat rule are
 * properties of the room's sequence, and because a track name a client could choose is a
 * URL every phone in the plan would load.
 *
 * `kitchens` is here and is not a section a plan has: standing in your own kitchen is not
 * something a room does together, so it has no shared state and never will.
 *
 * A pool of one track loops it; an empty pool is silence, where a section sits until its
 * tracks arrive.
 */
export const POOLS: Record<string, string[]> = {
  kitchens: [
    "/music/title-1.mp3",
    "/music/title-2.mp3",
    "/music/title-3.mp3",
    "/music/title-4.mp3",
  ],
  pick: ["/pick.mp3"],
  buy: [
    "/music/buy-1.mp3",
    "/music/buy-2.mp3",
    "/music/buy-3.mp3",
    "/music/buy-4.mp3",
  ],
  cook: ["/music/cook-1.mp3", "/music/cook-2.mp3", "/music/cook-3.mp3"],
  // Its own tracks are still to come; empty until then.
  joy: [],
};

const NONE: string[] = [];

/** The lone device's pool for a route, or nothing where a route has no music. */
export function poolFor(pathname: string): string[] {
  return POOLS[pathname.split("/")[1]] ?? NONE;
}

/**
 * A random track from `pool`, avoiding `exclude` so a pool of several does not repeat
 * back to back. The rule relaxes when there is nothing else to honour it with — a
 * single-track pool, or one whose remaining tracks are all `exclude` — and returns what
 * there is, so a lone track just keeps playing.
 *
 * The **lone device's** shuffle. Its counterpart for a room is `music::choose` on the
 * server, which takes its roll as an argument so every branch of it is pinned rather
 * than sampled at.
 */
export function pickFrom(pool: string[], exclude: string | null): string {
  const others = pool.filter((track) => track !== exclude);
  const options = others.length ? others : pool;
  return options[Math.floor(Math.random() * options.length)];
}

/** The four legs of a meal — the sections a plan has a shared soundtrack for. */
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
 * know which plan is underway, and they know it in the two different ways the app
 * already stores it. This reads both rather than inventing a third:
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
 * **`null` is the whole solo story**: no plan, no socket, no reconcile loop, and the
 * lone device's random pick untouched. A pick with no channel in the path (`/pick`, the
 * page that starts one) is one of those, and so is a decision stashed by a build from
 * before plans carried a channel.
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

/**
 * **Where the room is in its track**, in seconds, on this device.
 *
 * The one piece of arithmetic the whole feature rests on. `started_at` is a shared
 * instant and `Date.now()` is this device's clock, so the two are only comparable once
 * the device's own measured drift is taken out of the second one — `toShared`, spelled
 * out here rather than imported, because reading it in place is what makes the direction
 * checkable: a phone five minutes fast reads a *larger* `now`, so its offset must come
 * **off** it or that phone would think the room is five minutes further into the song
 * than it is, and seek there.
 *
 * Negative when the room's track starts a moment from now — a rollover instant that
 * arrived before this device's clock reached it. The caller plays from the top; a second
 * later the arithmetic is positive and ordinary.
 */
export function expectedPosition(
  startedAt: number,
  offsetMs: number,
  nowLocal: number,
): number {
  return (nowLocal - offsetMs - startedAt) / 1000;
}

// ---- self-healing (#214) ----------------------------------------------------

/**
 * How often a device checks itself against the room, in ms.
 *
 * Playback is physical — a buffering stall, a throttled background tab, a slow decode, a
 * phone that slept — so desync is a condition to notice and repair rather than a state
 * to settle into. A second is well inside the {@link SEEK_S} threshold at any plausible
 * rate of drift, so nothing audible has time to accumulate between two checks, and it is
 * a rate a sleeping tab costs nothing for.
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
 * How long a device waits before repeating a report the room did not answer, in ms.
 *
 * A report is refused in silence, as every refusal on this socket is (#179/#180) — a
 * watcher's, or one that lost the race and whose answering frame was lost. Without a
 * bound the device would raise the same refused report on every check for as long as the
 * track stayed ended. Five seconds is slow enough to be nothing and fast enough that a
 * genuinely lost frame costs one gap, not a section of silence.
 */
export const ADVANCE_RETRY_MS = 5000;

/**
 * **What a device should do to be where the room is.** Strictly local convergence: no
 * branch here pauses, seeks or reshuffles anybody else.
 */
export type Repair =
  /** Playing the wrong track — a rollover happened while this device was asleep or
   * disconnected, or it has just arrived. Load the room's track at the room's position;
   * the same rehydrate story as every other piece of plan state (#202). */
  | { kind: "load"; track: string; position: number }
  /** Too far out to heal by ear: a stall, or a wake from sleep. Jump. */
  | { kind: "seek"; position: number }
  /** Drifting: run slightly fast or slow until it converges. `rate` is under 1 when this
   * device is *ahead* of the room. */
  | { kind: "nudge"; rate: number }
  /** In sync. Play at 1×. */
  | { kind: "hold" }
  /** The room's track has ended in the shared timeline, so the room needs moving on.
   * Any seated member's device may say so; exactly one of them wins the compare-and-set
   * on the server and the rest are told what it chose. */
  | { kind: "advance" };

function clamp(value: number, limit: number): number {
  return Math.max(-limit, Math.min(limit, value));
}

/**
 * **Compare where this device is against where the room is, and answer with the repair.**
 *
 * The truth is already there — #212's track event on #208's drift-compensated timeline —
 * so this is a comparison rather than a negotiation, and it runs periodically rather than
 * once at track start, because everything that breaks playback happens in the middle.
 *
 * The branches are in the order they are because each rules the next one's question out:
 *
 * 1. **A different track** is not drift at all; nothing about a position on the wrong
 *    song is meaningful, so it is answered first.
 * 2. **Past the end** — the room's timeline has run off this track, so there is no
 *    position to converge on and the answer is to move the room on, not to seek. A
 *    duration that is not a finite number yet (metadata still loading) is not an answer,
 *    so this branch is not taken on one.
 * 3. **Far out** → seek. 4. **Out** → nudge. 5. Otherwise hold.
 *
 * `position` is never negative: a rollover instant a moment in the future plays from the
 * top rather than seeking to a place no track has.
 */
export function reconcile(
  room: RoomTrack,
  playing: { track: string | null; currentTime: number; duration: number },
  expected: number,
): Repair {
  const position = Math.max(0, expected);
  if (playing.track !== room.track) {
    return { kind: "load", track: room.track, position };
  }
  if (Number.isFinite(playing.duration) && expected >= playing.duration) {
    return { kind: "advance" };
  }
  const drift = playing.currentTime - expected;
  if (Math.abs(drift) >= SEEK_S) return { kind: "seek", position };
  if (Math.abs(drift) > IN_SYNC_S) {
    // Ahead of the room ⇒ slow down. The sign is the whole of it: run this the other way
    // and every device races further from the room the harder it tries to catch up.
    return { kind: "nudge", rate: 1 - clamp(drift / CONVERGE_S, MAX_NUDGE) };
  }
  return { kind: "hold" };
}
