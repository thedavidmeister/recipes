<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { goto } from "$app/navigation";
  import { getWalk } from "$lib/walk";
  import { ApiError } from "$lib/client";
  import { createSession } from "$lib/session";
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

  // A lapsed session 401s the walk. The layout stopped polling `/api/me` once it
  // had a session, so without this the page would sit on an error whose "try
  // again" just 401s forever. Re-asking the session drops the whole app back to
  // Login, the only real recovery.
  $effect(() => {
    if (walk.error instanceof ApiError && walk.error.status === 401) {
      queryClient.invalidateQueries({ queryKey: ["session"] });
    }
  });

  function again() {
    queryClient.invalidateQueries({ queryKey: ["walk"] });
  }

  // Start a shared session (#20) and hand off to its live room. Deciding together
  // is still picking — the multiplayer mode of this same wander.
  let starting = $state(false);
  async function decideTogether() {
    starting = true;
    try {
      const channel = await createSession();
      await goto(`/pick/${channel}`);
    } catch {
      // A failed start (a lapsed session, say) just re-enables the button; the
      // walk below keeps working solo.
      starting = false;
    }
  }
</script>

<div class="flex items-center justify-between gap-4 pt-6">
  <p class="font-display text-sm text-stone-500">Deciding with others?</p>
  <button
    onclick={decideTogether}
    disabled={starting}
    class="rounded-pill font-display inline-flex items-center gap-2 bg-pesto-500 px-4 py-2 text-sm font-medium text-cream-50 transition-colors hover:bg-pesto-500/90 disabled:opacity-50"
  >
    {starting ? "Starting…" : "Decide together"}
  </button>
</div>

<Walk
  {status}
  stops={walk.data}
  busy={walk.isFetching}
  error={walk.error instanceof Error ? walk.error.message : undefined}
  onAgain={again}
/>
