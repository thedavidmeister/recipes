<script lang="ts">
  import Skeleton from "./Skeleton.svelte";
  import RowLink from "./RowLink.svelte";
  import type { KitchenMeal, KitchensStatus, MealAddition } from "$lib/types";

  /**
   * The meals in this kitchen (#207), newest first, each a way back into its plan.
   *
   * A plan's channel used to be the only door to it: lose the URL and you lose the
   * meal, which for a plan that runs days (#202/#204) is most of them. The kitchen is
   * where a member finds them again, so every row links to `/pick/{channel}` — tapping
   * a lobby seats you (arrival is joining, #96), tapping a started one carries on where
   * your deal left off, and tapping a decided one goes to what the room settled on.
   *
   * Three states, in the plan's own words rather than the wire's:
   *
   * - **gathering** — the lobby is open and people are still arriving, so the row says
   *   how many are in *so far*. That number is going up, which is the whole difference
   *   between this state and the next one.
   * - **deciding** — the swiping has begun and the roster is closed (#96), so the count
   *   is final: it is the number a recipe has to win over.
   * - **decided** — the outcome is a server fact (#205), so the row names the recipe
   *   itself. Nothing here recomputes a winner from a tally; the title is what the
   *   server joined onto the decision it recorded.
   *
   * A plan everybody walked out of is a deleted row (#169), so there is no "ended"
   * state to render and nothing is padded to stand in for one — such a meal simply is
   * not listed.
   *
   * Starting a meal is **not** here. The kitchen already has one way to do that — the
   * "Let's cook!" button above — and a second button would be two places to press for
   * one act, so the empty state points at the one that exists instead.
   */
  interface Props {
    status: KitchensStatus;
    /** This kitchen's meals, newest first, as the server ordered them. */
    meals?: KitchenMeal[];
    error?: string;
  }

  let { status, meals = [], error }: Props = $props();

  // Display-only sentence-casing over an ASCII vocabulary; the wire stays lowercase —
  // the same treatment the lobby gives the same words (#114).
  const mealLabel = (t: string) => t.charAt(0).toUpperCase() + t.slice(1);

  /** "Dinner with dessert & side" — the plan named in one line, the way it was made. */
  const mealWords = (meal: KitchenMeal) => {
    const label = mealLabel(meal.meal_type);
    return meal.additions.length
      ? `${label} with ${listAdditions(meal.additions)}`
      : label;
  };

  const listAdditions = (list: MealAddition[]) =>
    list.length <= 1
      ? (list[0] ?? "")
      : `${list.slice(0, -1).join(", ")} & ${list[list.length - 1]}`;

  /** "3 people" / "1 person" — a count of people, said as people. */
  const people = (n: number) => `${n} ${n === 1 ? "person" : "people"}`;
</script>

<section class="mt-8 border-t border-stone-200 pt-6">
  <h2 class="font-display text-lg font-medium text-stone-900">Meals</h2>

  {#if status === "error"}
    <p class="mt-2 text-sm text-stone-600">
      {error ?? "Couldn't load this kitchen's meals."}
    </p>
  {:else if status === "pending"}
    <div class="mt-3"><Skeleton /></div>
  {:else if meals.length === 0}
    <!-- Nothing to show, and nothing invented to fill the space: this kitchen has
         planned no meals. It points back at the one button that starts one rather
         than offering a second. -->
    <p class="mt-2 text-sm text-stone-600">
      No meals here yet — "Let's cook!" above starts the first one.
    </p>
  {:else}
    <ul class="mt-4 flex flex-col gap-2">
      {#each meals as meal (meal.channel_id)}
        <li>
          <RowLink href="/pick/{meal.channel_id}">
            <span class="flex flex-col gap-0.5">
              <span>{mealWords(meal)}</span>
              {#if meal.decided}
                <!-- What the room is having. Read at full strength, because it is the
                     answer rather than a note about one. -->
                <span class="text-sm text-stone-900">{meal.decided.title}</span>
              {:else if meal.started}
                <span class="text-sm text-stone-500">
                  Deciding — {people(meal.deciders)}
                </span>
              {:else}
                <span class="text-sm text-stone-500">
                  Gathering — {people(meal.deciders)} so far
                </span>
              {/if}
            </span>
          </RowLink>
        </li>
      {/each}
    </ul>
  {/if}
</section>
