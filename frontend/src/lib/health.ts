import { ApiError, apiFetch } from "./client";
import type { HealthStats } from "./types";

/**
 * Fetch the admin health snapshot (`GET /api/admin/health`).
 *
 * The stats are computed server-side — the browser never queries the internal
 * `runs`/`raw_imports` tables; it asks and renders, the same split the walk uses.
 *
 * The endpoint is session-gated then admin-gated, so two statuses are meaningful
 * to the caller: **401** (session lapsed) and **403** (logged in, but not the
 * admin). Both throw an `ApiError` carrying the status so the page can tell "log in
 * again" from "this page is not for you" rather than matching on a message.
 */
export async function fetchHealth(): Promise<HealthStats> {
  const res = await apiFetch("/api/admin/health");
  if (!res.ok) {
    throw new ApiError(
      res.status,
      res.status === 401
        ? "Your session has expired."
        : res.status === 403
          ? "This page is for the admin."
          : `could not load health (${res.status})`,
    );
  }
  return (await res.json()) as HealthStats;
}
