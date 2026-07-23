<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import {
    getKitchen,
    stashCurrentKitchen,
    forgetCurrentKitchen,
  } from "$lib/kitchens";
  import type { KitchensStatus } from "$lib/types";
  import Kitchen from "$lib/components/Kitchen.svelte";
  import { goto } from "$app/navigation";
  import { createPick } from "$lib/pick";

  /** One kitchen (#72). The id comes from the route, so there is no selection state. */
  const id = $derived(page.params.id ?? "");

  const detail = createQuery(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
  }));

  const status = $derived<KitchensStatus>(
    detail.isError ? "error" : detail.isPending ? "pending" : "ready",
  );


  /** Start a meal plan for this kitchen; its lobby is where the deciders gather. */
  async function planMeal() {
    const channel = await createPick(undefined, id);
    await goto(`/pick/${channel}`);
  }

  /**
   * Opening a kitchen is how you switch to it, and only a switch is remembered: land
   * on your primary and the stored one is cleared, so the app goes back to assuming
   * the default rather than holding a preference you did not express.
   *
   * The meal flow reads this to scope pick/buy/cook to a kitchen (a follow-up to #72).
   */
  $effect(() => {
    if (!detail.data) return;
    if (detail.data.is_primary) forgetCurrentKitchen();
    else stashCurrentKitchen(detail.data.id);
  });
</script>

<Kitchen
  {status}
  onPlan={planMeal}
  kitchen={detail.data}
  error={detail.error instanceof Error ? detail.error.message : undefined}
/>
