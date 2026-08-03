<script lang="ts">
  import KitchenBackdrop from "./KitchenBackdrop.svelte";
  import Kitchen from "./Kitchen.svelte";
  import KitchenMeals from "./KitchenMeals.svelte";
  import { kitchenDetail, kitchenMeals } from "$lib/fixtures";

  /**
   * A kitchen as the page actually composes it: the backdrop, the layout container, the
   * kitchen, and its meals in the slot the page fills.
   *
   * The two halves are separate components because they are separate reads — the room
   * moves slowly, its meals do not — so neither declares what the pair looks like
   * together. This does, and it is the only place the fence can see the meals section
   * against the photograph, under the button whose name its empty state says out loud.
   * It reuses `KitchenBackdrop` and the real components rather than restating any of
   * them, so it cannot drift from the page (the same job `KitchensPreview` does for the
   * kitchens landing).
   */
</script>

<KitchenBackdrop />

<div class="mx-auto max-w-2xl px-4">
  <Kitchen status="ready" kitchen={kitchenDetail()}>
    {#snippet meals()}
      <KitchenMeals status="ready" meals={kitchenMeals()} />
    {/snippet}
  </Kitchen>
</div>
