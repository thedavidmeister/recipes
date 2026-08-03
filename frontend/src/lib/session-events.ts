import type { Voter } from "./pick";

/**
 * **The browser half of the app's shared framework for time-sensitive event handling**
 * (#208) — the counterpart to `backend/src/events.rs`, which carries the full reasoning.
 *
 * Three things live here, and none of them is about cooking:
 *
 * - the **envelope** a client raises an event in — what happened, and the initiator's
 *   own clock at the moment it happened;
 * - this browser's half of the **clock measurement** — answering the server's ping, and
 *   holding the offset the server measured for this connection;
 * - **translating instants** between this device's clock and the shared timeline.
 *
 * ## Why the initiator's clock is on the wire at all
 *
 * Because the tap is the event. A phone that stalls for a second between the tap and
 * the frame reaching the server must not shorten the room's timer by a second, and a
 * server receipt is exactly that mistake with a confident name on it. So the initiator
 * stamps the instant, and everybody else renders relative to it (ruled, #208).
 *
 * A stamp from a clock that is wrong is worse than useless, which is why the same
 * framework measures the clock. Over the session socket the server sends its own
 * reading (`time_ping`), this side answers with that reading plus its own
 * ({@link pongFor}), and the server times the return and hands back what it measured
 * (`time_sync`). Both sides then work off **one** number — the server subtracts it to
 * normalise an incoming event, this side adds it back to render a shared instant
 * ({@link toLocal}) — so the initiator's own countdown and every peer's agree exactly
 * rather than drifting apart by two separate estimates of the same thing.
 *
 * ## Pure, and its own module
 *
 * Nothing here fetches, and nothing here reads `$env` — the `$lib/shopping` and
 * `$lib/roster` split (#176/7, enforced by `lint:env`), so a story and a unit test can
 * both reach it. `$lib/pick` owns the socket and calls into this; the `Voter` type comes
 * back the other way as an `import type`, which is erased.
 *
 * ## The events that have not migrated yet
 *
 * Votes and shopping ticks predate this and still travel as their own frames. Nothing
 * here is timer-shaped — {@link SessionEvent} is a union to add a member to, and the
 * envelope, the measurement and the translation know only about instants — so migrating
 * them is adding a variant and a sender, not reworking this.
 */

/** A recipe key, on the events that name one. */
interface RecipeRef {
  source: string;
  id: string;
}

/**
 * **What happened** — the payload of an event, exactly as the backend's
 * `events::SessionEvent` deserialises it.
 *
 * Deliberately thin: a kind carries what the server cannot work out for itself, and
 * nothing more. `timer_start` names a step and **no duration**, because the duration is
 * the recipe's and the server reads it from the corpus — the initiator owns *when*,
 * never *how long*. There is nowhere on this wire for one phone to make a 30-minute
 * braise three seconds long for everybody else's screen.
 *
 * Who raised it is not here either. That is the authenticated session, which this side
 * could only ever claim rather than prove.
 */
export type SessionEvent =
  | ({ kind: "timer_start"; step: number } & RecipeRef)
  | ({ kind: "timer_dismiss"; step: number } & RecipeRef);

/** An event on its way to the server: the payload, plus when the initiator did it. */
export interface EventFrame {
  type: "event";
  /** The initiator's own clock at the moment of the action, in unix ms. */
  at: number;
  event: SessionEvent;
}

/** This browser's answer to a `time_ping`: the server's reading, plus its own. */
export interface TimePongFrame {
  type: "time_pong";
  server_ms: number;
  client_ms: number;
}

/**
 * Raise an event **now** — stamped with this device's clock, because this device is
 * where the tap happened.
 *
 * `now` is a parameter so a test can state the instant rather than race one. Callers
 * pass nothing.
 */
export function raise(event: SessionEvent, now: number = Date.now()): EventFrame {
  return { type: "event", at: now, event };
}

/**
 * Answer a clock ping: echo the server's reading back untouched, and add this device's.
 *
 * Echoing is what lets the server time the round trip against the send it belongs to —
 * an answer to a ping two ticks ago would otherwise be read as a 30-second round trip.
 * Answering immediately is the whole contract: any delay this side adds lands in the
 * measured round trip and is charged half to the offset.
 */
export function pongFor(
  serverMs: number,
  now: number = Date.now(),
): TimePongFrame {
  return { type: "time_pong", server_ms: serverMs, client_ms: now };
}

/**
 * A shared-timeline instant, on this device's clock.
 *
 * `offsetMs` is what the server measured this connection's clock to be doing —
 * `client - server` — so adding it back is the exact inverse of the normalisation the
 * server did on the way in. A device whose clock is five minutes fast gets a deadline
 * five minutes later *on its own clock*, which is the same real moment as everybody
 * else's, which is the point.
 *
 * **`0` until the first `time_sync` lands** (a round trip, on connect). That is the
 * honest starting assumption rather than a guess, and it is why a countdown is rendered
 * from the offset rather than from a duration frozen at connect: the first sync corrects
 * it in place, without a reload.
 */
export function toLocal(sharedMs: number, offsetMs: number): number {
  return sharedMs + offsetMs;
}

/** This device's clock reading, on the shared timeline — the inverse of {@link toLocal}. */
export function toShared(localMs: number, offsetMs: number): number {
  return localMs - offsetMs;
}

/**
 * One step's shared countdown, as the room holds it. Mirrors `timers::RunningTimer`.
 *
 * `started_at` and `deadline` are **shared-timeline** instants — translate them with
 * {@link toLocal} before comparing them to `Date.now()`, never before.
 */
export interface RunningTimer {
  /** Which step of the recipe's stored reading (#74) is running. */
  step: number;
  /** The initiator's tap, normalised into the shared timeline. */
  started_at: number;
  /** `started_at` plus the step's duration, as the corpus states it. */
  deadline: number;
  /** Who started it — a shared pot has whose hand on it, like a ticked shopping line. */
  started_by: Voter;
}
