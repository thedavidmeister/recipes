<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { getKitchen, renameKitchen } from "$lib/kitchens";
  import KitchenRename from "$lib/components/KitchenRename.svelte";

  /** Renaming a kitchen (#72). It puts you back in the kitchen you renamed. */
  const id = $derived(page.params.id ?? "");
  const qc = useQueryClient();

  const detail = createQuery(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
    retry: false,
  }));

  let error = $state<string | null>(null);

  async function onRename(name: string) {
    error = null;
    try {
      const k = await renameKitchen(id, name);
      qc.setQueryData(["kitchen", k.id], k);
      await qc.invalidateQueries({ queryKey: ["kitchens"] });
      await goto(`/kitchens/${k.id}`);
    } catch (e) {
      error = e instanceof Error ? e.message : "could not rename the kitchen";
      throw e;
    }
  }
</script>

{#if detail.data}
  <KitchenRename
    current={detail.data.name}
    error={error ?? undefined}
    {onRename}
  />
{/if}
