<script lang="ts">
  import KitchenBackdrop from "./KitchenBackdrop.svelte";
  import Kitchens from "./Kitchens.svelte";
  import { kitchenList, kitchenDetail } from "$lib/fixtures";

  /**
   * The kitchens page as it actually composes: the backdrop, the layout container,
   * and the view over the top.
   *
   * It exists because the visual fence renders *components*, so nothing otherwise
   * covers the page chrome — whether the content stays legible against the scrim is
   * exactly the thing that can regress silently. It reuses `KitchenBackdrop` rather
   * than restating it, so it cannot drift from the page.
   */
</script>

<KitchenBackdrop />

<div class="mx-auto max-w-2xl px-4 pb-16">
  <Kitchens
    status="ready"
    kitchens={kitchenList()}
    selected={kitchenDetail()}
    inviteLink="https://recipes.lehlehleh.com/kitchens?join=a1b2c3d4e5f6a7b8"
  />
</div>
