<script lang="ts">
  import PickBackdrop from "./PickBackdrop.svelte";
  import Pick from "./Pick.svelte";
  import { recipeCards } from "$lib/fixtures";

  /**
   * The pick as it actually composes: the backdrop, the layout container, and the
   * swipe on top.
   *
   * The visual fence renders *components*, so a page's chrome is exactly what nothing
   * else covers — and legibility against a busy photograph is precisely the thing that
   * regresses without anyone noticing. It reuses `PickBackdrop` rather than restating
   * it, so it cannot drift from the page.
   */
</script>

<PickBackdrop />

<div class="mx-auto max-w-2xl px-4">
  <Pick
    status="swiping"
    card={recipeCards()[0]}
    participants={2}
    shareUrl="https://recipes.lehlehleh.com/pick/8f2a1c4e9b7d"
  />
</div>
