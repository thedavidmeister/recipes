<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { getKitchen, addEquipment, removeEquipment } from "$lib/kitchens";
  import type { KitchenDetail, KitchensStatus } from "$lib/types";
  import KitchenItems from "$lib/components/KitchenItems.svelte";

  /** A kitchen's equipment (#72) — its own page, so it is one idea. */
  const id = $derived(page.params.id ?? "");
  const qc = useQueryClient();

  const detail = createQuery(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
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
    run(() => addEquipment(id, item), "could not add that");
  const onRemove = (item: string) =>
    run(() => removeEquipment(id, item), "could not remove that");
</script>

<KitchenItems
  {status}
  title="Equipment"
  items={detail.data?.equipment}
  placeholder="Add equipment (blender, wok…)"
  backHref="/kitchens/{id}"
  error={detail.error instanceof Error ? detail.error.message : undefined}
  actionError={actionError ?? undefined}
  {onAdd}
  {onRemove}
/>
