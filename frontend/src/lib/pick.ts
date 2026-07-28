import { ApiError, apiFetch, backendUrl } from "./client";
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
        ? "Your session has expired."
        : `could not start a pick (${res.status})`,
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
    sql: "SELECT source, id, title, image, category, area, total_seconds, fully_timed FROM recipes WHERE source = ? AND id = ? LIMIT 1",
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
    };

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
  /** A full tally: sent on join and on every reconnect, so **replace**, don't merge. */
  onTally?: (participants: number, votes: TallyRow[]) => void;
  /** The roster size and whether the swiping has begun — on join, and on every
   * change to either. */
  onLobby?: (deciders: number, started: boolean) => void;
  /** One live vote from any peer (including this client's own echo). */
  onVote?: (voter: string, source: string, id: string, vote: boolean) => void;
  /** One recipe's shopping checklist, **whole** — someone ticked or unticked a
   * line, so this replaces the list rather than merging into it (#131). */
  onBuy?: (source: string, id: string, checks: RoomBuyCheck[]) => void;
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

  constructor(
    private readonly channel: string,
    private readonly handlers: PickHandlers,
  ) {}

  /** Open the socket (and keep it open across drops until [`stop`]). */
  start(): void {
    this.stopped = false;
    this.connect(true);
  }

  /** Send this client's yes/no on a recipe. Dropped silently if not connected —
   * the durable record is the server's, and the user can re-swipe on reconnect. */
  vote(source: string, id: string, vote: boolean): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ type: "vote", source, id, vote }));
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
      if (msg.type === "tally") {
        this.handlers.onTally?.(msg.participants, msg.votes);
      } else if (msg.type === "lobby") {
        this.handlers.onLobby?.(msg.deciders, msg.started);
      } else if (msg.type === "vote") {
        this.handlers.onVote?.(msg.voter, msg.source, msg.id, msg.vote);
      } else if (msg.type === "buy") {
        this.handlers.onBuy?.(msg.source, msg.id, msg.checks);
      }
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
}

function lobbyFailed(status: number, action: string): ApiError {
  return new ApiError(
    status,
    status === 401
      ? "Your session has expired."
      : `could not ${action} (${status})`,
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
  const res = await apiFetch(`/api/session/${encodeURIComponent(channel)}/cap`, {
    method: "POST",
    body: JSON.stringify({ max_total_seconds: cap }),
  });
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
