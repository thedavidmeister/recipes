<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { getBuyList, loadChecks, saveChecks } from "$lib/buy";
  import type { BuyStatus } from "$lib/types";
  import Buy from "$lib/components/Buy.svelte";

  /**
   * `buy` (#36) — the shopping checklist for the recipe the group picked.
   *
   * The step after `pick`: a pick decides on one recipe (consensus) and stashes it,
   * so this reads that decision and lists its ingredients to tick off. The page
   * owns the query and the checklist state; `Buy` renders. Read client-direct from
   * Turso (the corpus is public), so a lapsed session doesn't 401 it — the layout
   * already gates the shell.
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

  // The shopping checklist — which ingredients are already in the basket. Owned
  // here and persisted per recipe, so it survives a reload mid-shop.
  let checked = $state<Record<number, boolean>>({});

  // Load this recipe's ticks when it arrives (or changes).
  $effect(() => {
    const r = list.data;
    if (!r) {
      checked = {};
      return;
    }
    const map: Record<number, boolean> = {};
    for (const i of loadChecks(r.source, r.id)) map[i] = true;
    checked = map;
  });

  function toggle(index: number) {
    const r = list.data;
    if (!r) return;
    checked = { ...checked, [index]: !checked[index] };
    saveChecks(
      r.source,
      r.id,
      Object.keys(checked)
        .map(Number)
        .filter((i) => checked[i]),
    );
  }
</script>

<Buy
  {status}
  recipe={list.data}
  error={list.error instanceof Error ? list.error.message : undefined}
  {checked}
  onToggle={toggle}
/>
