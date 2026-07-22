<script lang="ts">
  import KitchenBackdrop from "./KitchenBackdrop.svelte";
  import KitchenList from "./KitchenList.svelte";
  import { kitchenList } from "$lib/fixtures";

  /**
   * The kitchens landing as it actually composes: the backdrop, the layout container,
   * and the view on top.
   *
   * The visual fence renders *components*, so nothing else covers the page chrome —
   * and whether the content stays legible against the backdrop is exactly what
   * regresses silently. It reuses `KitchenBackdrop` rather than restating it, so it
   * cannot drift from the page.
   */
</script>

<KitchenBackdrop />

<div class="mx-auto max-w-2xl px-4">
  <KitchenList status="ready" kitchens={kitchenList()} />
</div>
