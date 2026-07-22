import { ApiError, apiFetch, backendUrl } from "./client";
import { turso } from "./turso";
import type { RecipeCard } from "./types";

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

/** `POST /api/session` — start a pick, returning its shareable channel id. */
export async function createPick(
  filter?: string,
  kitchenId?: string,
): Promise<string> {
  const res = await apiFetch("/api/session", {
    method: "POST",
    body: JSON.stringify({
      filter: filter ?? null,
      kitchen_id: kitchenId ?? null,
    }),
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
    sql: "SELECT source, id, title, image, category, area FROM recipes WHERE source = ? AND id = ? LIMIT 1",
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
  };
}

/** A recipe's running tally in a pick — mirrors the backend `TallyRow`. */
export interface TallyRow {
  source: string;
  id: string;
  yes: number;
  no: number;
}

/** A frame the backend sends over the room. Mirrors `session::ServerMsg`. */
export type ServerMsg =
  | { type: "tally"; participants: number; votes: TallyRow[] }
  | { type: "lobby"; deciders: number; started: boolean }
  | { type: "vote"; voter: string; source: string; id: string; vote: boolean };

/** The connection's live state, surfaced so the UI can show "reconnecting…". */
export type ConnStatus = "connecting" | "open" | "reconnecting" | "closed";

/** How the page reacts to the socket — wired to Svelte `$state` at the call site. */
export interface PickHandlers {
  /** A full tally: sent on join and on every reconnect, so **replace**, don't merge. */
  onTally: (participants: number, votes: TallyRow[]) => void;
  /** The roster size and whether the swiping has begun — on join, and on every
   * change to either. */
  onLobby: (deciders: number, started: boolean) => void;
  /** One live vote from any peer (including this client's own echo). */
  onVote: (
    voter: string,
    source: string,
    id: string,
    vote: boolean,
  ) => void;
  onStatus: (status: ConnStatus) => void;
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
    this.handlers.onStatus(first ? "connecting" : "reconnecting");
    const ws = new WebSocket(this.url());
    this.ws = ws;

    ws.onopen = () => {
      this.backoffMs = 500;
      this.handlers.onStatus("open");
    };
    ws.onmessage = (e) => {
      let msg: ServerMsg;
      try {
        msg = JSON.parse(e.data as string) as ServerMsg;
      } catch {
        return;
      }
      if (msg.type === "tally") {
        this.handlers.onTally(msg.participants, msg.votes);
      } else if (msg.type === "lobby") {
        this.handlers.onLobby(msg.deciders, msg.started);
      } else if (msg.type === "vote") {
        this.handlers.onVote(msg.voter, msg.source, msg.id, msg.vote);
      }
    };
    ws.onclose = () => {
      this.ws = null;
      if (this.stopped) {
        this.handlers.onStatus("closed");
        return;
      }
      this.handlers.onStatus("reconnecting");
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
  host: string;
  started: boolean;
  voters: Voter[];
}

function lobbyFailed(status: number, action: string): ApiError {
  return new ApiError(
    status,
    status === 401 ? "Your session has expired." : `could not ${action} (${status})`,
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
  const res = await apiFetch(`/api/session/${encodeURIComponent(channel)}/join`, {
    method: "POST",
  });
  if (!res.ok) throw lobbyFailed(res.status, "join this meal plan");
  return (await res.json()) as Lobby;
}

/** Close the lobby and start swiping. Host only. */
export async function startPlan(channel: string): Promise<Lobby> {
  const res = await apiFetch(`/api/session/${encodeURIComponent(channel)}/start`, {
    method: "POST",
  });
  if (!res.ok) throw lobbyFailed(res.status, "start this meal plan");
  return (await res.json()) as Lobby;
}
