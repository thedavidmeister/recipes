<script lang="ts">
  import { resource } from "$lib/resource";
  import { page } from "$app/state";
  import { getKitchen } from "$lib/kitchens";
  import KitchenSettings from "$lib/components/KitchenSettings.svelte";

  /**
   * A kitchen's settings hub (#117): the way through to rename, invite, equipment and
   * pantry, pulled off the kitchen page so that page is about being in the kitchen.
   * The id comes from the route, so there is no selection state.
   */
  const id = $derived(page.params.id ?? "");

  const detail = resource(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
  }));
</script>

<KitchenSettings
  status={detail.status}
  {id}
  kitchen={detail.data}
  error={detail.error}
/>
