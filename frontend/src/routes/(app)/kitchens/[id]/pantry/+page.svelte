<script lang="ts">
  import { resource } from "$lib/resource";
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import {
    getKitchen,
    addPantry,
    removePantry,
    pantryVocabulary,
  } from "$lib/kitchens";
  import type { KitchenDetail, KitchensStatus } from "$lib/types";
  import KitchenItems from "$lib/components/KitchenItems.svelte";

  /** A kitchen's pantry (#72) — its own page, so it is one idea. */
  const id = $derived(page.params.id ?? "");
  const qc = useQueryClient();

  const detail = resource(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
  }));

  /**
   * What may be stocked at all. The pantry picks from what recipes cook with and the
   * server refuses anything else, so a free field would be offering a failure.
   */
  const known = resource(() => ({
    queryKey: ["pantry-vocabulary"],
    queryFn: pantryVocabulary,
  }));


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
  status={detail.status}
  title="Pantry"
  items={detail.data?.pantry}
  options={known.data ?? []}
  placeholder="Add to the pantry (rice, eggs…)"
  backHref="/kitchens/{id}"
  error={detail.error}
  actionError={actionError ?? undefined}
  {onAdd}
  {onRemove}
/>
