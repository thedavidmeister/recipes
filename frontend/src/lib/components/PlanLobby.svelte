<script lang="ts">
  import Skeleton from "./Skeleton.svelte";
  import Panel from "./Panel.svelte";
  import Button from "./Button.svelte";
  import type { Voter } from "$lib/pick";
  import {
    MEAL_ADDITIONS,
    MEAL_TYPES,
    type MealAddition,
    type MealType,
  } from "$lib/types";
  import QrCode from "./QrCode.svelte";

  /**
   * The lobby a meal plan starts in (#20, #72): the people who will decide gather,
   * and only then does the swiping begin.
   *
   * The roster it builds is the whole point. "Everyone agreed" needs an *everyone*,
   * and the two ways to guess at one are both wrong: who has voted so far reads as
   * one person until a friend swipes, and who happens to be connected turns a reload
   * into somebody leaving. Joining is a thing you do, so the answer is simply the
   * list.
   *
   * Only whoever started it can begin — otherwise a guest arriving late could close
   * the door on the person still inviting people.
   */
  interface Props {
    status: "pending" | "error" | "ready";
    voters?: Voter[];
    /** Kitchen members not yet in — the host can add them without a link (#72). */
    candidates?: Voter[];
    /** Which meal this plans (#114) — the heading, so voters know what they are
     * deciding. Undefined only while the lobby is still loading. */
    mealType?: MealType;
    /** What comes with the meal (#114) — shown to everyone under the heading. */
    additions?: MealAddition[];
    /** The shareable URL that seats whoever opens it. */
    inviteLink?: string;
    /** Whether the viewer is the one who started the plan. */
    host?: boolean;
    error?: string;
    onStart?: () => void;
    /** Add a kitchen member by id. Host only. */
    onSeat?: (userId: string) => void;
    /** Name which meal the plan is for. Host only, while the lobby is open. */
    onMealType?: (mealType: MealType) => void;
    /** Name what comes with it — the whole chosen set each time. Host only,
     * while the lobby is open. */
    onAdditions?: (additions: MealAddition[]) => void;
  }

  let {
    status,
    voters = [],
    candidates = [],
    mealType,
    additions = [],
    inviteLink,
    host = false,
    error,
    onStart,
    onSeat,
    onMealType,
    onAdditions,
  }: Props = $props();

  const name = (v: Voter) =>
    v.username ? `@${v.username}` : v.telegram_user_id;

  // Display-only sentence-casing over an ASCII vocabulary; the wire stays lowercase.
  const mealLabel = (t: string) => t.charAt(0).toUpperCase() + t.slice(1);

  // "with dessert & drink" — additions as a quiet prose list.
  const listAdditions = (list: MealAddition[]) =>
    list.length <= 1
      ? (list[0] ?? "")
      : `${list.slice(0, -1).join(", ")} & ${list[list.length - 1]}`;

  // A tap toggles one addition in or out of the chosen set; the parent gets the
  // whole set, mirroring the wire (a set each time, never a delta).
  const toggleAddition = (a: MealAddition) =>
    onAdditions?.(
      additions.includes(a)
        ? additions.filter((x) => x !== a)
        : [...additions, a],
    );

  let copied = $state(false);

  async function copyInvite() {
    if (!inviteLink) return;
    try {
      await navigator.clipboard.writeText(inviteLink);
      copied = true;
    } catch {
      // Clipboard blocked — the link is on screen to copy by hand.
    }
  }
</script>

<div class="pt-32 pb-16">
  <Panel>
    <!-- "Dinner plan" reads as a plan; "Meal plan" is the placeholder while the
         lobby is still loading. -->
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="bg-pesto-500 size-2.5 rounded-full" aria-hidden="true"
      ></span>
      {mealType ? `${mealLabel(mealType)} plan` : "Meal plan"}
    </p>
    {#if additions.length}
      <!-- The secondary tier, quietly: the room is deciding a dinner; the dessert
           and drinks come with it. -->
      <p class="mt-1 text-sm text-stone-500">
        with {listAdditions(additions)}
      </p>
    {/if}

    {#if status === "error"}
      <p class="mt-4 text-sm text-stone-600">
        {error ?? "Couldn't open this meal plan."}
      </p>
    {:else if status === "pending"}
      <div class="mt-4"><Skeleton /></div>
    {:else}
      <p class="mt-4 text-sm text-stone-600">
        {voters.length === 1
          ? "Just you so far. Start whenever you like, or invite someone first."
          : `${voters.length} deciding. Everyone here has to agree before a recipe wins.`}
      </p>

      {#if host}
        <!-- The choice is made once, up front, by the host; everyone else reads
             it off the heading. The vocabulary is closed, so the row of pills IS
             the whole set — nothing to type, nothing to get wrong. -->
        <p class="mt-6 mb-3 text-xs text-stone-500">Which meal</p>
        <div class="flex flex-wrap gap-2">
          {#each MEAL_TYPES as t (t)}
            <button
              type="button"
              aria-pressed={t === mealType}
              onclick={() => onMealType?.(t)}
              class="rounded-pill px-3 py-1 text-sm {t === mealType
                ? 'bg-cocoa-500 text-cream-50'
                : 'border-cocoa-500 text-cocoa-500 border'}"
            >
              {mealLabel(t)}
            </button>
          {/each}
        </div>

        <!-- The secondary tier: several may come with the meal, or none, so these
             pills toggle rather than choose — and they read quieter than the meal
             row (stone outline, not cocoa) because they are not what the pick
             decides. -->
        <p class="mt-6 mb-3 text-xs text-stone-500">What comes with it</p>
        <div class="flex flex-wrap gap-2">
          {#each MEAL_ADDITIONS as a (a)}
            <button
              type="button"
              aria-pressed={additions.includes(a)}
              onclick={() => toggleAddition(a)}
              class="rounded-pill px-3 py-1 text-sm {additions.includes(a)
                ? 'bg-cocoa-500 text-cream-50'
                : 'border-stone-300 text-stone-600 border'}"
            >
              {mealLabel(a)}
            </button>
          {/each}
        </div>
      {/if}

      <p class="mt-6 mb-3 text-xs text-stone-500">Who's deciding</p>
      <ul class="flex flex-col gap-1.5">
        {#each voters as v (v.telegram_user_id)}
          <li class="font-display text-stone-900">{name(v)}</li>
        {/each}
      </ul>

      {#if host && candidates.length}
        <p class="mt-8 mb-3 text-xs text-stone-500">In this kitchen</p>
        <ul class="flex flex-col gap-2">
          {#each candidates as c (c.telegram_user_id)}
            <li class="flex items-center justify-between">
              <span class="font-display text-stone-900">{name(c)}</span>
              <button
                type="button"
                onclick={() => onSeat?.(c.telegram_user_id)}
                class="rounded-pill border-cocoa-500 text-cocoa-500 border px-3 py-1 text-sm"
              >
                Add
              </button>
            </li>
          {/each}
        </ul>
      {/if}

      {#if inviteLink}
        <p class="mt-8 mb-3 text-xs text-stone-500">Invite someone to decide</p>
        <div class="flex flex-col items-center gap-3">
          <QrCode value={inviteLink} label="Scan to join this meal plan" />
          <button
            type="button"
            onclick={copyInvite}
            class="rounded-pill border-cocoa-500 text-cocoa-500 border px-3 py-1 text-sm"
          >
            {copied ? "Copied" : "Copy invite link"}
          </button>
        </div>
      {/if}

      {#if host}
        <div class="mt-8">
          <Button onclick={onStart} dot="pesto">Start</Button>
        </div>
      {:else}
        <p class="mt-8 text-sm text-stone-500">
          Waiting for whoever started this to begin.
        </p>
      {/if}
    {/if}
  </Panel>
</div>
