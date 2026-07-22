<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { goto, replaceState } from "$app/navigation";
  import { listKitchens, createKitchen, joinKitchen } from "$lib/kitchens";
  import type { KitchensStatus } from "$lib/types";
  import KitchenList from "$lib/components/KitchenList.svelte";

  /**
   * `kitchens` (#72) — the kitchens you're in. Opening one is a navigation, so this
   * page holds no selection state at all; the URL does that from here on.
   */
  const qc = useQueryClient();

  const list = createQuery(() => ({
    queryKey: ["kitchens"],
    queryFn: listKitchens,
    retry: false,
  }));

  const status = $derived<KitchensStatus>(
    list.isError ? "error" : list.isPending ? "pending" : "ready",
  );

  let actionError = $state<string | null>(null);

  async function onCreate(name: string) {
    actionError = null;
    try {
      const k = await createKitchen(name);
      await qc.invalidateQueries({ queryKey: ["kitchens"] });
      qc.setQueryData(["kitchen", k.id], k);
      await goto(`/kitchens/${k.id}`);
    } catch (e) {
      actionError = e instanceof Error ? e.message : "could not create the kitchen";
      throw e;
    }
  }

  // A shareable invite is a `?join=<token>` link: redeem it once on arrival, drop the
  // spent param, and go straight into the kitchen it opened.
  const attempted = new Set<string>();
  $effect(() => {
    const token = page.url.searchParams.get("join");
    if (!token || attempted.has(token)) return;
    attempted.add(token);
    joinKitchen(token)
      .then(async (k) => {
        await qc.invalidateQueries({ queryKey: ["kitchens"] });
        qc.setQueryData(["kitchen", k.id], k);
        const url = new URL(page.url);
        url.searchParams.delete("join");
        replaceState(url, page.state);
        await goto(`/kitchens/${k.id}`);
      })
      .catch((e) => {
        actionError = e instanceof Error ? e.message : "could not join that kitchen";
      });
  });
</script>

<KitchenList
  {status}
  kitchens={list.data}
  error={list.error instanceof Error ? list.error.message : undefined}
  actionError={actionError ?? undefined}
  {onCreate}
/>
