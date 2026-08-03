<script lang="ts">
  import { resource } from "$lib/resource";
  import { createQuery } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import {
    getKitchen,
    kitchenMeals,
    stashCurrentKitchen,
    forgetCurrentKitchen,
  } from "$lib/kitchens";
  import type { KitchensStatus } from "$lib/types";
  import Kitchen from "$lib/components/Kitchen.svelte";
  import KitchenMeals from "$lib/components/KitchenMeals.svelte";
  import { goto } from "$app/navigation";
  import { createPick } from "$lib/pick";

  /** One kitchen (#72). The id comes from the route, so there is no selection state. */
  const id = $derived(page.params.id ?? "");

  const detail = resource(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
  }));

  /**
   * The meals planned here (#207). A query of its own rather than a field on the
   * kitchen: plans move while the room sits still — somebody joins a lobby, a plan
   * decides — and the members and shelves above should not wait on that read, nor be
   * re-read every time a meal does.
   */
  const mealList = resource(() => ({
    queryKey: ["kitchen-meals", id],
    queryFn: () => kitchenMeals(id),
  }));

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
  status={detail.status}
  onPlan={planMeal}
  kitchen={detail.data}
  error={detail.error}
>
  {#snippet meals()}
    <KitchenMeals
      status={mealList.status}
      meals={mealList.data ?? []}
      error={mealList.error}
    />
  {/snippet}
</Kitchen>
