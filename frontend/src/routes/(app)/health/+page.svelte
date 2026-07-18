<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
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
  const session = createQuery(() => ({
    queryKey: ["session"],
    queryFn: me,
    retry: false,
  }));
  const isAdmin = $derived(session.data?.is_admin === true);

  const health = createQuery(() => ({
    queryKey: ["health"],
    queryFn: fetchHealth,
    enabled: isAdmin,
    retry: false,
    refetchInterval: 30_000,
  }));

  const status = $derived<HealthStatus>(
    session.data && !isAdmin
      ? "forbidden"
      : health.isError
        ? health.error instanceof ApiError && health.error.status === 403
          ? "forbidden"
          : "error"
        : health.data
          ? "ready"
          : "pending",
  );
  const error = $derived(
    health.error instanceof Error ? health.error.message : undefined,
  );

  // A 401 from the health poll means the session lapsed since the page loaded.
  // Re-check the session so the `(app)` layout re-gates to the login screen,
  // rather than leaving a dashboard error card up for a signed-out visitor. (403
  // is the *admin* gate and stays on the page as `forbidden`.)
  const queryClient = useQueryClient();
  $effect(() => {
    if (health.error instanceof ApiError && health.error.status === 401) {
      queryClient.invalidateQueries({ queryKey: ["session"] });
    }
  });
</script>

<HealthDashboard {status} stats={health.data} {error} />
