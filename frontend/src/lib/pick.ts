import { ApiError, apiFetch, backendUrl } from "./client";
import { applyFrame } from "./frames";
import { pongFor, raise } from "./session-events";
import type { RunningTimer, SessionEvent } from "./session-events";
import { turso } from "./turso";
import type { MealAddition, MealType, RecipeCard } from "./types";

/**
 * The live, shared machinery of `pick` (#20).
 *
 * A pick is a swipe-and-vote everyone in it shares. Three things live here:
 * starting a pick, fetching a single card for peer-injection, and the live
 * [`PickClient`] over the backend's WebSocket (`/api/session/{channel}/ws`). The
 * backend is the source of truth (Turso); this is a thin live layer that reconnects
 * and rehydrates, so a dropped socket — or Render's spin-down — is a blip, not lost
 * votes.
 */

/** `POST /api/session` — start a pick, returning its shareable channel id.
 * `maxTotalSeconds` caps the plan to recipes estimated at that long or less
 * (#80); `null` asks for "Any", and omitting it takes the backend's default —
 * half an hour (#163). The host can still move it in the lobby, until start.
 */
export async function createPick(
  filter?: string,
  kitchenId?: string,
  maxTotalSeconds?: number | null,
): Promise<string> {
  // The cap is left OUT of the body rather than sent as null when the caller
  // names none, and the two are not the same thing (#163): the backend reads an
  // absent cap as the 30-minute default and an explicit null as "Any". Collapsing
  // them with `?? null` — which is what this used to do — would start every plan
  // unbounded again while the code still looked like it asked for the default.
  const asked: Record<string, string | number | null> = {
    filter: filter ?? null,
    kitchen_id: kitchenId ?? null,
  };
  if (maxTotalSeconds !== undefined) {
    asked.max_total_seconds = maxTotalSeconds;
  }
  const res = await apiFetch("/api/session", {
    method: "POST",
    body: JSON.stringify(asked),
  });
  if (!res.ok) {
    throw new ApiError(
      res.status,
      res.status === 401
        ? "You've been signed out. Sign in again to carry on."
        : `Couldn't start a pick (${res.status}).`,
    );
  }
  const body = (await res.json()) as { channel_id: string };
  return body.channel_id;
}

/**
 * One recipe's card, read **client-direct from Turso** (read-only token).
 *
 * A peer's vote names a recipe by `(source, id)`, but a client that has not walked
 * to it yet has no card to render. Rather than fatten every vote frame — or add a
 * backend read per vote — the client fetches the one card it is missing, straight
 * from the corpus it already has read access to. Returns `null` for an id that is
 * not in the corpus, so a bogus vote injects nothing.
 */
export async function fetchCard(
  source: string,
  id: string,
): Promise<RecipeCard | null> {
  const rs = await turso().execute({
    // Named columns, never `SELECT *` — the row is read by name below, and a
    // wildcard would hand back whatever order the table happens to have (#109).
    sql: "SELECT source, id, title, image, category, area, total_seconds, fully_timed, kcal, kcal_complete, servings FROM recipes WHERE source = ? AND id = ? LIMIT 1",
    args: [source, id],
  });
  const row = rs.rows[0];
  if (!row) return null;
  const str = (v: unknown): string | null => (v == null ? null : String(v));
  return {
    source: String(row.source),
    id: String(row.id),
    title: String(row.title),
    image: str(row.image),
    category: str(row.category),
    area: str(row.area),
    // A peer-injected card carries its time estimate too (#84): a card is a card
    // however it reached the deck, and one that arrived this way must not silently
    // lose a field the walk's cards have. `null` stays `null` — unknown, not zero.
    total_seconds: row.total_seconds == null ? null : Number(row.total_seconds),
    // And the mark that estimate is rendered with (#158). Dropping it here would
    // show a peer-injected card as a floor (`23 min+`) while the identical card
    // reached by walking showed `~23 min` — the same recipe contradicting itself
    // across the deck. SQLite has no boolean type, so this arrives as 0/1.
    fully_timed: Number(row.fully_timed) !== 0,
    // The calorie estimate travels with it for the same reason (#162), and all three
    // of its columns do: without `servings` a per-serving badge would silently become
    // a whole-recipe one, and without `kcal_complete` a floor would render as an
    // estimate. A card is a card however it reached the deck.
    kcal: row.kcal == null ? null : Number(row.kcal),
    // NOT NULL DEFAULT 0 in the schema, and 0/1 on the wire like `fully_timed`.
    kcal_complete: Number(row.kcal_complete) !== 0,
    servings: row.servings == null ? null : Number(row.servings),
  };
}

/** A recipe's running tally in a pick — mirrors the backend `TallyRow`. */
export interface TallyRow {
  source: string;
  id: string;
  yes: number;
  no: number;
  /** Who said yes, by telegram id — so a card can wear the colours of the people
   * who liked it even on the first frame after a reconnect (#131/#145). */
  yes_voters: string[];
}

/**
 * What a plan decided (#201). Mirrors `session::DecidedRecipe`.
 *
 * The server's **record**, not a client's reading of the tally. The win condition is
 * evaluated inside the write that stores it, so holding one of these is the same thing
 * as the pick being over — nothing on this side recomputes it, and nothing on this side
 * is allowed to disagree with it. That is the whole of why it moved.
 */
export interface Decided {
  source: string;
  id: string;
  /** Unix seconds, from the database's clock. */
  decided_at: number;
}

/**
 * **The plan is cooking** (#211). Mirrors `session::Cooking`.
 *
 * The server's record of the moment somebody tapped "Let's cook!", not a client's
 * reading of anything — the same standing as {@link Decided} one step later in the arc,
 * and it arrives the same two ways: live to the room, and on every connect.
 *
 * It names no recipe. A plan cooks what it decided, which this side already holds, and a
 * second copy of it here would be a second answer to what the room is having.
 */
export interface Cooking {
  /** When the cook started, on the **shared timeline** — translate with `toLocal` and
   * this connection's offset before comparing it to `Date.now()`. */
  started_at: number;
  /** Whose tap started it. */
  started_by: Voter;
}

/** A frame the backend sends over the room. Mirrors `session::ServerMsg`. */
export type ServerMsg =
  | { type: "tally"; participants: number; votes: TallyRow[] }
  | { type: "lobby"; deciders: number; started: boolean }
  | { type: "vote"; voter: string; source: string; id: string; vote: boolean }
  | {
      type: "buy";
      source: string;
      id: string;
      checks: RoomBuyCheck[];
    }
  | { type: "left"; voter: Voter; ended: boolean }
  | { type: "decided"; source: string; id: string; decided_at: number }
  | { type: "time_ping"; server_ms: number }
  | { type: "time_sync"; offset_ms: number; rtt_ms: number }
  | { type: "timers"; source: string; id: string; timers: RunningTimer[] }
  | { type: "cooking"; started_at: number; started_by: Voter };

/**
 * One ticked line as the room announces it. Structurally `BuyCheck` from `$lib/buy`,
 * stated here rather than imported so this module stays the wire's own description
 * and does not import from a module that imports it back.
 *
 * Exactly one of `by` and `pantry` is set: a person got it, or the plan's kitchen
 * already had it (#156). The server's schema enforces that; see `session::BuyCheck`.
 */
export interface RoomBuyCheck {
  index: number;
  by: Voter | null;
  pantry: string | null;
}

/** The connection's live state, surfaced so the UI can show "reconnecting…". */
export type ConnStatus = "connecting" | "open" | "reconnecting" | "closed";

/**
 * How a page reacts to the socket — wired to Svelte `$state` at the call site.
 *
 * Every handler is optional because the room now serves two rooms' worth of
 * screens: the pick listens to votes and the lobby, `buy` listens to the shopping
 * list, and neither should have to write empty functions for the other's traffic.
 * The frames themselves are unchanged — a client simply ignores what it did not
 * ask about.
 */
export interface PickHandlers {
  /** A full tally: sent on join and on every reconnect, so **replace**, don't merge.
   *
   * `participants` is `COUNT(DISTINCT voter_id)` — how many people have swiped at
   * all, which is **not** the number a recipe has to win over and must not decide a
   * pick (#181): one person's first yes arrives here as `participants: 1, yes: 1`.
   *
   * Since #201 it decides nothing here at all, and neither does the roster: the win
   * condition is evaluated on the server and its answer arrives on
   * {@link onDecided}. This is the running score. */
  onTally?: (participants: number, votes: TallyRow[]) => void;
  /** The roster size and whether the swiping has begun — on join, and on every
   * change to either. `deciders` is the roster, which the server decides against
   * (#201); this side shows it. */
  onLobby?: (deciders: number, started: boolean) => void;
  /** **The plan decided** (#201) — the one frame that ends a pick.
   *
   * Sent to the room the instant the deciding vote lands, and again to every socket on
   * connect, so a member whose browser was closed when the last yes arrived is *told*
   * what was decided rather than re-deriving it. A watcher (#180) is told the same way:
   * the frame goes to the room, not to the roster.
   *
   * There is no client-side counterpart and deliberately so. Two evaluators of one win
   * condition are two answers to "what did we pick", and the server holds the roster
   * and the votes. */
  onDecided?: (decided: Decided) => void;
  /** One live vote from any peer (including this client's own echo). */
  onVote?: (voter: string, source: string, id: string, vote: boolean) => void;
  /** One recipe's shopping checklist, **whole** — someone ticked or unticked a
   * line, so this replaces the list rather than merging into it (#131). */
  onBuy?: (source: string, id: string, checks: RoomBuyCheck[]) => void;
  /** Somebody left the plan (#96) — always from its lobby, because that is the only
   * place the roster moves. The `lobby` frame beside this one already carries the
   * smaller roster, so `voter` is who, for a screen that wants to name them.
   *
   * `ended` is the part nothing else can say: they were the last, so the plan itself
   * is gone and there is no roster left to send. */
  onLeft?: (voter: Voter, ended: boolean) => void;
  /** The server's clock, asking for this device's — the event framework's drift
   * measurement (`$lib/session-events`). {@link PickClient} answers this itself and a
   * page never sees it; it is on the interface because the reading of the wire is a
   * pure function in `$lib/frames` and every frame has to land somewhere there. */
  onTimePing?: (serverMs: number) => void;
  /** **What the server measured this connection's clock to be doing** — how far ahead
   * of the shared timeline it reads, and the round trip that estimate came from.
   *
   * {@link PickClient} records it (see {@link PickClient.clockOffsetMs}); a page only
   * needs this if it wants to re-render the moment the estimate moves, which a page
   * showing a countdown does. */
  onTimeSync?: (offsetMs: number, rttMs: number) => void;
  /** One recipe's shared cook timers, **whole** (#208) — on connect, and on every start
   * and dismiss anyone in the plan makes.
   *
   * Whole rather than a delta, exactly like {@link onBuy}: a client that missed a frame
   * is corrected by the next one instead of drifting. Instants are on the **shared
   * timeline** — translate with `toLocal` and this connection's offset before comparing
   * them to `Date.now()`. */
  onTimers?: (source: string, id: string, timers: RunningTimer[]) => void;
  /** **The plan is cooking** (#211) — the frame that moves the room to the stove.
   *
   * Sent to the room the instant somebody taps "Let's cook!", and again to every socket
   * on connect, so a member who dropped — or who came back into the plan through their
   * kitchen (#207) — lands at the stove rather than on a shopping list nobody is
   * shopping from. A watcher (#180/#200) is told the same way and comes along read-only:
   * the frame goes to the room, not to the roster.
   *
   * It is what moves the **initiator** too. `PickClient.startCook` navigates nothing on
   * its own, exactly as {@link onDecided} is what ends a pick rather than the swipe that
   * completed it — one path, so the person who tapped cannot arrive somewhere the room
   * did not. */
  onCooking?: (cooking: Cooking) => void;
  onStatus?: (status: ConnStatus) => void;
}

/**
 * A resilient WebSocket to a pick's room.
 *
 * Reconnects with exponential backoff (a dropped socket after Render's 5-min idle
 * close, or a full spin-down, is expected), and the server re-sends the whole tally
 * on every (re)connect — so recovery is automatic: the page just replaces its tally
 * on each `onTally`. Callback-based rather than reactive so the reactivity lives in
 * the page (the framework-native place), and this stays a plain, testable client.
 */
export class PickClient {
  private ws: WebSocket | null = null;
  private stopped = false;
  private backoffMs = 500;
  private readonly maxBackoffMs = 10_000;
  /**
   * What the server has measured **this connection's** clock to be doing, in ms
   * (`client - server`) — the event framework's recorded offset for this participant.
   *
   * `0` until the first `time_sync` lands, which is one round trip after connect, and
   * back to `0` on a reconnect because the offset describes a socket: a new connection
   * is measured afresh rather than inheriting how wrong this phone used to be.
   */
  private offsetMs = 0;

  constructor(
    private readonly channel: string,
    private readonly handlers: PickHandlers,
  ) {}

  /** Open the socket (and keep it open across drops until [`stop`]). */
  start(): void {
    this.stopped = false;
    this.connect(true);
  }

  /**
   * This connection's recorded clock offset, for translating shared-timeline instants
   * with `toLocal` (`$lib/session-events`). A page that shows a countdown reads it on
   * every frame rather than caching it — the estimate is refreshed for the life of the
   * socket, and a countdown rendered off a stale one drifts exactly as far as the clock
   * did.
   */
  get clockOffsetMs(): number {
    return this.offsetMs;
  }

  /** Send this client's yes/no on a recipe — **an event through the framework** since
   * #209, so the swipe carries the instant it was swiped.
   *
   * Dropped silently if not connected, exactly as it was when it had its own frame: the
   * durable record is the server's, and the user can re-swipe on reconnect. */
  vote(source: string, id: string, vote: boolean): void {
    this.event({ kind: "vote", source, id, vote });
  }

  /** Tick one line of the meal's shopping list off, or put it back (#131/#209).
   *
   * Was a `POST` that answered with the whole list; it is an event now, and the answer
   * is the `buy` frame the server sends **the room** — which is the same list, reaching
   * the tapper and everybody shopping beside them in one message instead of two. */
  tick(source: string, id: string, index: number, checked: boolean): void {
    this.event({ kind: "buy_tick", source, id, index, checked });
  }

  /**
   * Start the room's cook (#211) — the shopping list's "Let's cook!", raised rather than
   * followed.
   *
   * **This navigates nothing.** What moves a screen is the `cooking` frame the server
   * sends the whole room ({@link PickHandlers.onCooking}), which reaches the person who
   * tapped by exactly the same path as everybody else — the {@link PickHandlers.onDecided}
   * arrangement one step later in the arc, and for the same reason: two ways to arrive
   * are two chances for the initiator to be somewhere the room is not.
   *
   * No recipe travels with it. The plan cooks what it decided, and the server holds that.
   */
  startCook(): void {
    this.event({ kind: "cook_started" });
  }

  /**
   * **Raise an event through the framework** (#208) — stamped with this device's clock
   * at the moment of the call, because this device is where the action happened.
   *
   * Dropped silently if the socket is not open, like a vote: the durable record is the
   * server's, and a tap that never left is a tap to make again. Deliberately *not*
   * queued for the reconnect — a queued timer start would fire minutes late carrying an
   * instant that the server's sanity bound would then clamp, which is a worse lie than
   * a button that plainly did nothing.
   */
  event(event: SessionEvent): void {
    this.send(raise(event));
  }

  /** One frame out, if there is a socket to put it on. */
  private send(frame: unknown): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(frame));
    }
  }

  /** Close for good — no reconnect. */
  stop(): void {
    this.stopped = true;
    this.ws?.close();
    this.ws = null;
  }

  private url(): string {
    // ws(s):// mirrors http(s):// of the backend origin.
    const base = backendUrl().replace(/^http/, "ws");
    return `${base}/api/session/${encodeURIComponent(this.channel)}/ws`;
  }

  private connect(first: boolean): void {
    this.handlers.onStatus?.(first ? "connecting" : "reconnecting");
    const ws = new WebSocket(this.url());
    this.ws = ws;
    // A new socket is a new participant as far as the clock measurement is concerned;
    // the server measures it afresh, and rendering off the old number until it does
    // would be rendering off a connection that no longer exists.
    this.offsetMs = 0;

    ws.onopen = () => {
      this.backoffMs = 500;
      this.handlers.onStatus?.("open");
    };
    ws.onmessage = (e) => {
      let msg: ServerMsg;
      try {
        msg = JSON.parse(e.data as string) as ServerMsg;
      } catch {
        return;
      }
      // The reading of the wire lives in `$lib/frames`, where a test can reach it:
      // an unrecognised frame is dropped in silence on purpose, so a branch that
      // never fires looks exactly like a server that never sent one, and `decided`
      // is the frame a pick ends on.
      //
      // The two clock frames are answered and recorded **here** rather than by the
      // page: measuring this connection's drift is the socket's own business, and a
      // page that had to remember to answer a ping is a page that could forget and
      // silently render off an unmeasured clock. A page still hears about the estimate
      // (`onTimeSync`) — it just never has to keep the exchange going.
      applyFrame(msg, {
        ...this.handlers,
        onTimePing: (serverMs) => {
          // Immediately, and with no work in between: any delay this side adds lands
          // in the measured round trip and is charged half to the offset.
          this.send(pongFor(serverMs));
          this.handlers.onTimePing?.(serverMs);
        },
        onTimeSync: (offsetMs, rttMs) => {
          this.offsetMs = offsetMs;
          this.handlers.onTimeSync?.(offsetMs, rttMs);
        },
      });
    };
    ws.onclose = () => {
      this.ws = null;
      if (this.stopped) {
        this.handlers.onStatus?.("closed");
        return;
      }
      this.handlers.onStatus?.("reconnecting");
      const wait = this.backoffMs;
      this.backoffMs = Math.min(this.backoffMs * 2, this.maxBackoffMs);
      setTimeout(() => {
        if (!this.stopped) this.connect(false);
      }, wait);
    };
    // A socket error is always followed by close; reconnect is handled there.
    ws.onerror = () => {};
  }
}

/** A person in a meal plan. Mirrors `session::Voter`. */
export interface Voter {
  telegram_user_id: string;
  username: string | null;
}

/** A plan's lobby. Mirrors `session::LobbyView`. */
export interface Lobby {
  channel_id: string;
  kitchen_id: string | null;
  /** Which meal this plans (#114). Every plan has one; dinner until the host says. */
  meal_type: MealType;
  /** What comes with it (#114) — each at most once, in vocabulary order. */
  additions: MealAddition[];
  host: string;
  started: boolean;
  /** The plan's total-time cap in seconds (#80); null = "Any". */
  max_total_seconds: number | null;
  /** Whether we know what this plan's kitchen owns (#82). The walk is always limited
   * to what the kitchen can make; `false` means its equipment is unrecorded — a gap,
   * not a claim that it owns nothing — so nothing limits the deck. */
  voters: Voter[];
  /** Kitchen members not yet deciding — the host can add any without a link (#72). */
  candidates: Voter[];
  /** What this plan decided (#201), or null while its deck is still running.
   *
   * The same record the `decided` frame carries, on the one HTTP answer that describes
   * the whole plan — so a page that has read the lobby knows the pick is over without
   * waiting for its socket, and a lobby read can never be silent about it. */
  decided: Decided | null;
}

function lobbyFailed(status: number, action: string): ApiError {
  return new ApiError(
    status,
    status === 401
      ? "You've been signed out. Sign in again to carry on."
      : `Couldn't ${action} (${status}).`,
  );
}

/** The lobby: who is deciding, and whether it has begun. */
export async function getLobby(channel: string): Promise<Lobby> {
  const res = await apiFetch(`/api/session/${encodeURIComponent(channel)}`);
  if (!res.ok) throw lobbyFailed(res.status, "open this meal plan");
  return (await res.json()) as Lobby;
}

/** Join a plan as a decider. Refused once the swiping has begun. */
export async function joinLobby(channel: string): Promise<Lobby> {
  const res = await apiFetch(
    `/api/session/${encodeURIComponent(channel)}/join`,
    {
      method: "POST",
    },
  );
  if (!res.ok) throw lobbyFailed(res.status, "join this meal plan");
  return (await res.json()) as Lobby;
}

/** What a departure left behind (#96). Mirrors `session::Departure`. */
export interface Departure {
  channel_id: string;
  /** The kitchen the plan was for — where leaving puts you back. Null for a plan
   * started outside one, in which case the client falls back to your own. */
  kitchen_id: string | null;
  /** Whether that was the last person, so the plan is gone rather than smaller. */
  plan_ended: boolean;
  /** Who holds the plan now; null when it ended. Different from you exactly when
   * you were the host and it passed on. */
  host: string | null;
}

/**
 * Leave a meal plan (#96) — the inverse of {@link joinLobby}, on the same path.
 *
 * A **lobby** act, exactly like joining: the roster closes at the start in both
 * directions, so once the swiping has begun the set of people a recipe has to win
 * over is fixed and this is refused with the same 400 every other lobby write gives.
 *
 * If you started the plan it passes to the next person in the room; if you were the
 * last one in it the plan ends.
 */
export async function leavePlan(channel: string): Promise<Departure> {
  const res = await apiFetch(
    `/api/session/${encodeURIComponent(channel)}/join`,
    { method: "DELETE" },
  );
  if (!res.ok) throw lobbyFailed(res.status, "leave this meal plan");
  return (await res.json()) as Departure;
}

/** Add a kitchen member to the plan without a link. Host only, before it starts. */
export async function seatMember(
  channel: string,
  userId: string,
): Promise<Lobby> {
  const res = await apiFetch(
    `/api/session/${encodeURIComponent(channel)}/seat`,
    { method: "POST", body: JSON.stringify({ user_id: userId }) },
  );
  if (!res.ok) throw lobbyFailed(res.status, "add that person");
  return (await res.json()) as Lobby;
}

/** Name which meal the plan is for (#114). Host only, before it starts. */
export async function setMealType(
  channel: string,
  mealType: MealType,
): Promise<Lobby> {
  const res = await apiFetch(
    `/api/session/${encodeURIComponent(channel)}/meal-type`,
    { method: "POST", body: JSON.stringify({ meal_type: mealType }) },
  );
  if (!res.ok) throw lobbyFailed(res.status, "change the meal");
  return (await res.json()) as Lobby;
}

/** Name what comes with the meal (#114) — the whole chosen set each time.
 * Host only, before it starts. */
export async function setAdditions(
  channel: string,
  additions: MealAddition[],
): Promise<Lobby> {
  const res = await apiFetch(
    `/api/session/${encodeURIComponent(channel)}/additions`,
    { method: "POST", body: JSON.stringify({ additions }) },
  );
  if (!res.ok) throw lobbyFailed(res.status, "change the additions");
  return (await res.json()) as Lobby;
}

/** Set (or lift, with `null`) the plan's time cap in seconds (#80). Host only,
 * and only while the lobby is open — the cap freezes when the swiping starts. */
export async function setPlanCap(
  channel: string,
  cap: number | null,
): Promise<Lobby> {
  const res = await apiFetch(
    `/api/session/${encodeURIComponent(channel)}/cap`,
    {
      method: "POST",
      body: JSON.stringify({ max_total_seconds: cap }),
    },
  );
  if (!res.ok) throw lobbyFailed(res.status, "set the time cap");
  return (await res.json()) as Lobby;
}

/** Close the lobby and start swiping. Host only. */
export async function startPlan(channel: string): Promise<Lobby> {
  const res = await apiFetch(
    `/api/session/${encodeURIComponent(channel)}/start`,
    {
      method: "POST",
    },
  );
  if (!res.ok) throw lobbyFailed(res.status, "start this meal plan");
  return (await res.json()) as Lobby;
}
