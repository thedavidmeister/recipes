<script lang="ts">
  import KitchenBackdrop from "$lib/components/KitchenBackdrop.svelte";

  /**
   * Every kitchens route sits in the same room, so the backdrop lives here rather
   * than being repeated per page.
   */
  let { children } = $props();
</script>

<KitchenBackdrop />
{@render children()}
