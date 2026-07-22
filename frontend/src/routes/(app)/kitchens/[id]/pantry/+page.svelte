<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { getKitchen, addPantry, removePantry } from "$lib/kitchens";
  import type { KitchenDetail, KitchensStatus } from "$lib/types";
  import KitchenItems from "$lib/components/KitchenItems.svelte";

  /** A kitchen's pantry (#72) — its own page, so it is one idea. */
  const id = $derived(page.params.id ?? "");
  const qc = useQueryClient();

  const detail = createQuery(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
    retry: false,
  }));

  const status = $derived<KitchensStatus>(
    detail.isError ? "error" : detail.isPending ? "pending" : "ready",
  );

  let actionError = $state<string | null>(null);

  function cache(k: KitchenDetail) {
    qc.setQueryData(["kitchen", k.id], k);
  }

  async function run(fn: () => Promise<KitchenDetail>, fallback: string) {
    actionError = null;
    try {
      cache(await fn());
    } catch (e) {
      actionError = e instanceof Error ? e.message : fallback;
      throw e;
    }
  }

  const onAdd = (item: string) =>
    run(() => addPantry(id, item), "could not add that");
  const onRemove = (item: string) =>
    run(() => removePantry(id, item), "could not remove that");
</script>

<KitchenItems
  {status}
  title="Pantry"
  items={detail.data?.pantry}
  placeholder="Add to the pantry (rice, eggs…)"
  backHref="/kitchens/{id}"
  error={detail.error instanceof Error ? detail.error.message : undefined}
  actionError={actionError ?? undefined}
  {onAdd}
  {onRemove}
/>
