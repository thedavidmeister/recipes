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

/** Join a kitchen by its invite token, as a guest. */
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
// Which kitchen the user has open, so it survives a reload. The meal flow will read
// this to scope pick/buy/cook to a kitchen (a follow-up to #72).

const CURRENT_KEY = "recipes:current-kitchen";

/** Remember the open kitchen. */
export function stashCurrentKitchen(id: string): void {
  try {
    localStorage.setItem(CURRENT_KEY, id);
  } catch {
    // No storage (private mode): the page just defaults to the first kitchen.
  }
}

/** The last-opened kitchen id, if any. */
export function currentKitchen(): string | null {
  try {
    return localStorage.getItem(CURRENT_KEY);
  } catch {
    return null;
  }
}
