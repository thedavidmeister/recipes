<script lang="ts">
  import { resource } from "$lib/resource";
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { getKitchen, addEquipment, removeEquipment } from "$lib/kitchens";
  import type { KitchenDetail, KitchensStatus } from "$lib/types";
  import KitchenItems from "$lib/components/KitchenItems.svelte";
  import { equipmentVocabulary } from "$lib/kitchens";

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
    run(() => addEquipment(id, item), "could not add that");
  const onRemove = (item: string) =>
    run(() => removeEquipment(id, item), "could not remove that");
</script>

<KitchenItems
  status={detail.status}
  title="Equipment"
  items={detail.data?.equipment}
  options={known.data ?? []}
  placeholder="Add equipment (blender, wok…)"
  backHref="/kitchens/{id}"
  error={detail.error}
  actionError={actionError ?? undefined}
  {onAdd}
  {onRemove}
/>
