<script lang="ts">
  import { resource } from "$lib/resource";
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { getKitchen, addEquipment, removeEquipment } from "$lib/kitchens";
  import type { KitchenDetail, KitchensStatus } from "$lib/types";
  import KitchenItems from "$lib/components/KitchenItems.svelte";
  import EquipmentAdvice from "$lib/components/EquipmentAdvice.svelte";
  import { equipmentVocabulary, equipmentAdvice } from "$lib/kitchens";

  /** A kitchen's equipment (#72) — its own page, so it is one idea. */
  const id = $derived(page.params.id ?? "");
  const qc = useQueryClient();

  const detail = resource(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
  }));

  /**
   * What may be owned at all. A kitchen picks from this and cannot invent an item
   * (#81) — the server refuses anything outside it, so offering a free field would be
   * offering a failure.
   */
  const known = resource(() => ({
    queryKey: ["equipment-vocabulary"],
    queryFn: equipmentVocabulary,
  }));

  /**
   * What to add next (#83). A query of its own rather than a field on the kitchen: it
   * is counted over the whole corpus, so it costs more than reading a list of items and
   * the list should not wait on it. Every change to the equipment invalidates it,
   * because the advice is about the gap and stocking the kitchen is what closes it.
   */
  const advice = resource(() => ({
    queryKey: ["equipment-advice", id],
    queryFn: () => equipmentAdvice(id),
  }));

  let actionError = $state<string | null>(null);

  function cache(k: KitchenDetail) {
    qc.setQueryData(["kitchen", k.id], k);
  }

  async function run(fn: () => Promise<KitchenDetail>, fallback: string) {
    actionError = null;
    try {
      cache(await fn());
      void qc.invalidateQueries({ queryKey: ["equipment-advice", id] });
    } catch (e) {
      actionError = e instanceof Error ? e.message : fallback;
      throw e;
    }
  }

  const onAdd = (item: string) =>
    run(() => addEquipment(id, item), "Couldn't add that.");
  const onRemove = (item: string) =>
    run(() => removeEquipment(id, item), "Couldn't remove that.");
</script>

<KitchenItems
  status={detail.status}
  title="Equipment"
  items={detail.data?.equipment}
  options={known.data ?? []}
  placeholder="Add equipment (blender, wok…)"
  backHref="/kitchens/{id}/settings"
  error={detail.error}
  actionError={actionError ?? undefined}
  {onAdd}
  {onRemove}
>
  {#snippet footer()}
    <EquipmentAdvice
      status={advice.status}
      advice={advice.data}
      error={advice.error}
    />
  {/snippet}
</KitchenItems>
