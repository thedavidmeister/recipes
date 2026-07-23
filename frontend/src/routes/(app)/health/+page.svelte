<script lang="ts">
  import { resource } from "$lib/resource";
  import { useQueryClient } from "@tanstack/svelte-query";
  import { me } from "$lib/auth";
  import { fetchHealth } from "$lib/health";
  import { ApiError } from "$lib/client";
  import type { HealthStatus } from "$lib/types";
  import HealthDashboard from "$lib/components/HealthDashboard.svelte";

  /**
   * The admin health dashboard. Session-gated by the `(app)` layout; admin-gated
   * both here (only fetch when `is_admin`) and — the real gate — server-side, which
   * 403s a non-admin. The page owns the query; `HealthDashboard` owns rendering.
   *
   * Shares the `["session"]` query with the layout, so `is_admin` costs no extra
   * request. Refetches on an interval so the numbers stay live while it is open.
   */
  const session = resource(() => ({
    queryKey: ["session"],
    queryFn: me,
  }));
  const isAdmin = $derived(session.data?.is_admin === true);

  const health = resource(() => ({
    queryKey: ["health"],
    queryFn: fetchHealth,
    enabled: isAdmin,
    refetchInterval: 30_000,
  }));

  /**
   * Loading is `resource`'s job; being allowed in is this page's. Only the `forbidden`
   * branch stays local, because it is the one part that is about admin-ness rather
   * than about a request — see the note in `$lib/resource`.
   */
  const status = $derived<HealthStatus>(
    (session.data && !isAdmin) ||
      (health.query.error instanceof ApiError &&
        health.query.error.status === 403)
      ? "forbidden"
      : health.status,
  );
  const error = $derived(health.error);

  // A 401 from the health poll means the session lapsed since the page loaded.
  // Re-check the session so the `(app)` layout re-gates to the login screen,
  // rather than leaving a dashboard error card up for a signed-out visitor. (403
  // is the *admin* gate and stays on the page as `forbidden`.)
  const queryClient = useQueryClient();
  $effect(() => {
    if (health.query.error instanceof ApiError && health.query.error.status === 401) {
      queryClient.invalidateQueries({ queryKey: ["session"] });
    }
  });
</script>

<HealthDashboard {status} stats={health.data} {error} />
