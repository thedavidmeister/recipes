<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { getWalk } from "$lib/walk";
  import type { WalkStatus } from "$lib/types";
  import Walk from "$lib/components/Walk.svelte";

  /**
   * What to eat — by *wandering* the corpus, not searching it (#47, #49).
   *
   * There is no query box: a schedule ingests every source server-side (#49), and
   * this page's job is to stroll what is already there. The walk itself runs on the
   * backend (`GET /api/walk`, the `recipe-walk` crate) — the browser asks and
   * renders, it never computes the walk, because that would mean shipping the graph
   * and strategy client-side, which the app deliberately does not do.
   *
   * The page owns the query; `Walk` owns rendering. Each fetch is a *fresh* journey,
   * so "walk again" just refetches — freshness is the point, not caching by identity.
   */
  const queryClient = useQueryClient();

  const walk = createQuery(() => ({
    queryKey: ["walk"],
    queryFn: () => getWalk(),
    // A walk is a snapshot in time, not data to keep fresh in the background — it
    // only ever changes because the user asked for another one.
    staleTime: Infinity,
    retry: false,
  }));

  const status = $derived<WalkStatus>(
    walk.isError ? "error" : walk.isPending ? "pending" : "ready",
  );

  function again() {
    queryClient.invalidateQueries({ queryKey: ["walk"] });
  }
</script>

<Walk
  {status}
  stops={walk.data}
  busy={walk.isFetching}
  error={walk.error instanceof Error ? walk.error.message : undefined}
  onAgain={again}
/>
