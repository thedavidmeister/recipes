<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { getBuyList } from "$lib/buy";
  import type { BuyStatus } from "$lib/types";
  import Buy from "$lib/components/Buy.svelte";

  /**
   * `buy` (#36) — what to get for the recipe the group picked.
   *
   * The step after `pick`: a pick decides on one recipe (consensus) and stashes it,
   * so this reads that decision and lists its ingredients. The page owns the query;
   * `Buy` renders. Read client-direct from Turso (the corpus is public), so a
   * lapsed session doesn't 401 it — the layout already gates the shell.
   */
  const list = createQuery(() => ({
    queryKey: ["buy"],
    queryFn: () => getBuyList(),
    staleTime: Infinity,
    retry: false,
  }));

  const status = $derived<BuyStatus>(
    list.isError ? "error" : list.isPending ? "pending" : "ready",
  );
</script>

<Buy
  {status}
  recipe={list.data}
  error={list.error instanceof Error ? list.error.message : undefined}
/>
