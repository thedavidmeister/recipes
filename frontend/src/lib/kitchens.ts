import { ApiError, apiFetch } from "./client";
import type { KitchenDetail, KitchenSummary } from "./types";

/**
 * Kitchens (#72): the durable shared space that scopes the meal flow. Unlike the
 * corpus reads (client-direct Turso), a kitchen is owned, per-user data — so every
 * call here goes through the session-gated backend via `apiFetch`.
 */

/** A friendlier message than a bare status; a 401 is a lapsed session, not a fault. */
function failed(status: number, action: string): ApiError {
  return new ApiError(
    status,
    status === 401 ? "Your session has expired." : `could not ${action} (${status})`,
  );
}

/** The kitchens the signed-in user belongs to. */
export async function listKitchens(): Promise<KitchenSummary[]> {
  const res = await apiFetch("/api/kitchens");
  if (!res.ok) throw failed(res.status, "load your kitchens");
  return (await res.json()) as KitchenSummary[];
}

/** One kitchen in full — members, equipment, pantry, invite. 403 if not a member. */
export async function getKitchen(id: string): Promise<KitchenDetail> {
  const res = await apiFetch(`/api/kitchens/${encodeURIComponent(id)}`);
  if (!res.ok) throw failed(res.status, "open this kitchen");
  return (await res.json()) as KitchenDetail;
}

/** Create a kitchen owned by the caller. */
export async function createKitchen(name: string): Promise<KitchenDetail> {
  const res = await apiFetch("/api/kitchens", {
    method: "POST",
    body: JSON.stringify({ name }),
  });
  if (!res.ok) throw failed(res.status, "create the kitchen");
  return (await res.json()) as KitchenDetail;
}

/** Rename a kitchen. Owner only; a guest gets a 403. */
export async function renameKitchen(
  id: string,
  name: string,
): Promise<KitchenDetail> {
  const res = await apiFetch(`/api/kitchens/${encodeURIComponent(id)}/name`, {
    method: "POST",
    body: JSON.stringify({ name }),
  });
  if (!res.ok) throw failed(res.status, "rename the kitchen");
  return (await res.json()) as KitchenDetail;
}

/**
 * Every piece of equipment any recipe asks for — the only things a kitchen may own
 * (#81).
 *
 * Empty until the corpus has been read, which is a real answer rather than a failure:
 * there is genuinely nothing legitimate to pick yet.
 */
export async function equipmentVocabulary(): Promise<string[]> {
  const res = await apiFetch("/api/equipment");
  if (!res.ok) throw failed(res.status, "load the equipment list");
  return (await res.json()) as string[];
}

/**
 * Every ingredient any recipe cooks with — the only things a pantry may hold (#72).
 *
 * Unlike the equipment list this has content today: it is drawn from the ingredient
 * readings, which have been enriched for a long time.
 */
export async function pantryVocabulary(): Promise<string[]> {
  const res = await apiFetch("/api/pantry");
  if (!res.ok) throw failed(res.status, "load the pantry list");
  return (await res.json()) as string[];
}

/** A freshly minted invite. Mirrors `kitchens::Invite`. */
export interface KitchenInvite {
  token: string;
  /** Unix seconds. The link stops working then — it is not a permalink. */
  expires_at: number;
}

/**
 * Mint an invite to a kitchen. Members only.
 *
 * Minted per ask rather than read off the kitchen, because there is nothing standing
 * to read: a link is good for two hours. Calling this is what makes one exist, so the
 * page asks when someone opens it.
 */
export async function mintInvite(id: string): Promise<KitchenInvite> {
  const res = await apiFetch(`/api/kitchens/${encodeURIComponent(id)}/invite`, {
    method: "POST",
  });
  if (!res.ok) throw failed(res.status, "make an invite");
  return (await res.json()) as KitchenInvite;
}

/** Join a kitchen by an invite, as a member like any other. */
export async function joinKitchen(token: string): Promise<KitchenDetail> {
  const res = await apiFetch("/api/kitchens/join", {
    method: "POST",
    body: JSON.stringify({ token }),
  });
  if (!res.ok) throw failed(res.status, "join that kitchen");
  return (await res.json()) as KitchenDetail;
}

/** Add/remove an equipment or pantry item; each returns the kitchen's fresh detail.
 * A remove passes the item as a query param (a DELETE with no body) so it survives
 * any proxy that strips DELETE bodies. */
async function mutateItem(
  kind: "equipment" | "pantry",
  method: "POST" | "DELETE",
  id: string,
  item: string,
): Promise<KitchenDetail> {
  const base = `/api/kitchens/${encodeURIComponent(id)}/${kind}`;
  const res =
    method === "DELETE"
      ? await apiFetch(`${base}?item=${encodeURIComponent(item)}`, { method })
      : await apiFetch(base, { method, body: JSON.stringify({ item }) });
  if (!res.ok) throw failed(res.status, `update the ${kind}`);
  return (await res.json()) as KitchenDetail;
}

export const addEquipment = (id: string, item: string) =>
  mutateItem("equipment", "POST", id, item);
export const removeEquipment = (id: string, item: string) =>
  mutateItem("equipment", "DELETE", id, item);
export const addPantry = (id: string, item: string) =>
  mutateItem("pantry", "POST", id, item);
export const removePantry = (id: string, item: string) =>
  mutateItem("pantry", "DELETE", id, item);

// --- the current kitchen (localStorage) ----------------------------------------
// Which kitchen the app is working in. Nothing is remembered until you *switch*: with
// no entry here the answer is your primary, which always exists, so there is no state
// in which the app does not know which kitchen it is in. The meal flow will read this
// to scope pick/buy/cook to a kitchen (a follow-up to #72).

const CURRENT_KEY = "recipes:current-kitchen";

/** Remember a switch to a kitchen that is not your primary. */
export function stashCurrentKitchen(id: string): void {
  try {
    localStorage.setItem(CURRENT_KEY, id);
  } catch {
    // No storage (private mode): every visit simply starts in the primary.
  }
}

/** Switch back to the primary — a kitchen that no longer opens must not be
 * returned to on the next visit. */
export function forgetCurrentKitchen(): void {
  try {
    localStorage.removeItem(CURRENT_KEY);
  } catch {
    // No storage (private mode): nothing was remembered to begin with.
  }
}

/** The kitchen switched to, or `null` for "the primary" — resolve it against a list
 * with {@link resolveKitchen}. */
export function currentKitchen(): string | null {
  try {
    return localStorage.getItem(CURRENT_KEY);
  } catch {
    return null;
  }
}

/**
 * The kitchen the app is working in: the one switched to, or the primary.
 *
 * A switch that no longer resolves — the kitchen was left, or never existed — falls
 * back to the primary rather than to nothing, because "no kitchen" is not a state the
 * app has. `undefined` only when the list itself is empty, which the server prevents.
 */
export function resolveKitchen(
  kitchens: KitchenSummary[],
): KitchenSummary | undefined {
  const switched = currentKitchen();
  return (
    (switched && kitchens.find((k) => k.id === switched)) ||
    kitchens.find((k) => k.is_primary) ||
    kitchens[0]
  );
}
