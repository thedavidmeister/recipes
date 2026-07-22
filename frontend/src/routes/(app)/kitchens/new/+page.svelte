<script lang="ts">
  import { useQueryClient } from "@tanstack/svelte-query";
  import { goto } from "$app/navigation";
  import { createKitchen } from "$lib/kitchens";
  import KitchenNew from "$lib/components/KitchenNew.svelte";

  /** Making a kitchen (#72). It lands you in the one you just made. */
  const qc = useQueryClient();

  let error = $state<string | null>(null);

  async function onCreate(name: string) {
    error = null;
    try {
      const k = await createKitchen(name);
      await qc.invalidateQueries({ queryKey: ["kitchens"] });
      qc.setQueryData(["kitchen", k.id], k);
      await goto(`/kitchens/${k.id}`);
    } catch (e) {
      error = e instanceof Error ? e.message : "could not create the kitchen";
      throw e;
    }
  }
</script>

<KitchenNew error={error ?? undefined} {onCreate} />
