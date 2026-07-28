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
  import UserName from "./UserName.svelte";
  import { userColour, userEdge, userTint } from "$lib/colour";

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
   *
   * Everyone in the roster wears their colour (#145) — the same one they have in
   * the kitchen — so "who is here" is a glance rather than a read.
   *
   * So does every choice on the plan. What the meal is, what comes with it and how
   * long there is are all the host's calls, so the chosen pill is washed in the
   * host's tint and carries their dot (#131): a guest reads "someone decided this,
   * and it was them" off the shape instead of a flat fill that could have come from
   * anywhere. Selected-ness itself stays on the cocoa outline and `aria-pressed`,
   * never on the colour — the tints are far too pale to be a control's boundary,
   * and that is exactly why they are safe to use for all six people.
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
    /** The plan's total-time cap in seconds (#80); null = "Any". A plan is born at
     * 1800 (#163), so a null here is a host who widened it, never a plan nobody has
     * got to yet. */
    cap?: number | null;
    /** Whether this plan is for a kitchen (#72). The deck is always limited to what
     * that kitchen can make (#82), so this decides whether there is anything to say. */
    /** Whether we know what that kitchen owns (#82). Not a setting anyone chose:
     * `false` means its equipment is unrecorded — a gap, not a claim that it owns
     * nothing — so nothing limits the deck. */
    /** The shareable URL that seats whoever opens it. */
    inviteLink?: string;
    /** Whether the viewer is the one who started the plan. */
    host?: boolean;
    /** Who started it, by telegram id — whose colour the plan's choices wear
     * (#131). Undefined only while the lobby is still loading. */
    hostId?: string;
    error?: string;
    onStart?: () => void;
    /** Add a kitchen member by id. Host only. */
    onSeat?: (userId: string) => void;
    /** Name which meal the plan is for. Host only, while the lobby is open. */
    onMealType?: (mealType: MealType) => void;
    /** Name what comes with it — the whole chosen set each time. Host only,
     * while the lobby is open. */
    onAdditions?: (additions: MealAddition[]) => void;
    /** Set (or lift, with null) the time cap. Host only, while the lobby is open. */
    onCap?: (cap: number | null) => void;
    /** Step out of the plan (#96). Everyone has it, the host included — a host who
     * cannot leave is trapped in their own plan. */
    onLeave?: () => void;
  }

  let {
    status,
    voters = [],
    candidates = [],
    mealType,
    additions = [],
    cap = null,
    inviteLink,
    host = false,
    hostId,
    error,
    onStart,
    onSeat,
    onMealType,
    onAdditions,
    onCap,
    onLeave,
  }: Props = $props();

  /**
   * What leaving would do, said before it is tapped rather than confirmed after.
   *
   * Leaving is cheap for a guest and consequential for the last person or the host,
   * so the warning belongs on the two cases that carry a consequence — a modal
   * asking "are you sure" on all three would tax the ordinary case to cover the
   * unusual one, and would say less.
   */
  const lastOneHere = $derived(voters.length <= 1);
  /** Who the plan would pass to: the next person in the room, which is the order
   * the roster above is already listed in. */
  const successor = $derived(
    host && !lastOneHere
      ? voters.find((v) => v.telegram_user_id !== hostId)
      : undefined,
  );

  /**
   * The presented time buckets (#80). A UI vocabulary, deliberately not a schema:
   * the backend stores plain seconds, so changing these is an edit here, not a
   * migration there.
   *
   * Shortest first, and "Any" last (#163). It is no cap at all — longer than two
   * hours, not shorter than thirty minutes — so at the head of the row it sat
   * where the eye scanning left-to-right expects the smallest. At the tail the row
   * is monotonic and reads as the scale it is, ending at "as long as it takes".
   */
  const BUCKETS: { label: string; seconds: number | null }[] = [
    { label: "30 min", seconds: 1800 },
    { label: "1 hour", seconds: 3600 },
    { label: "2 hours", seconds: 7200 },
    { label: "Any", seconds: null },
  ];

  /** "30 min" / "1 hour" / "2 hours" — or plain minutes for an off-bucket cap. */
  const capLabel = (s: number) =>
    s % 3600 === 0
      ? s === 3600
        ? "1 hour"
        : `${s / 3600} hours`
      : `${Math.round(s / 60)} min`;

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

  /**
   * The classes for a pill the host has chosen — their tint, and the cocoa outline
   * that says "chosen" at full contrast whatever their colour is. Without a host
   * to attribute it to (the lobby is still loading) it falls back to the plain
   * cocoa outline: an unattributed choice, not a wrong one.
   */
  const chosenPill = $derived(
    hostId
      ? `border ${userEdge(hostId)} text-stone-900 ${userTint(hostId)}`
      : "border-cocoa-500 border text-stone-900",
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
              class="rounded-pill flex items-center gap-2 px-3 py-1 text-sm {t ===
              mealType
                ? chosenPill
                : 'border-cocoa-500 text-cocoa-500 border'}"
            >
              {#if t === mealType && hostId}
                <span
                  class="size-2 shrink-0 rounded-full {userColour(hostId)}"
                  aria-hidden="true"
                ></span>
              {/if}
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
              class="rounded-pill flex items-center gap-2 px-3 py-1 text-sm {additions.includes(
                a,
              )
                ? chosenPill
                : 'border-stone-300 text-stone-600 border'}"
            >
              {#if additions.includes(a) && hostId}
                <span
                  class="size-2 shrink-0 rounded-full {userColour(hostId)}"
                  aria-hidden="true"
                ></span>
              {/if}
              {mealLabel(a)}
            </button>
          {/each}
        </div>

        <!-- The time cap (#80): one bound per plan, single-select like the meal
             row, frozen at start. The estimate it filters on is a lower bound, so
             the honesty note is part of the control, not a footnote. -->
        <p class="mt-6 mb-3 text-xs text-stone-500">How much time do you have?</p>
        <div class="flex flex-wrap gap-2">
          {#each BUCKETS as b (b.label)}
            <button
              type="button"
              aria-pressed={cap === b.seconds}
              onclick={() => onCap?.(b.seconds)}
              class="rounded-pill flex items-center gap-2 px-3 py-1 text-sm {cap ===
              b.seconds
                ? chosenPill
                : 'border-stone-300 text-stone-600 border'}"
            >
              {#if cap === b.seconds && hostId}
                <span
                  class="size-2 shrink-0 rounded-full {userColour(hostId)}"
                  aria-hidden="true"
                ></span>
              {/if}
              {b.label}
            </button>
          {/each}
        </div>
        {#if cap !== null}
          <p class="mt-2 text-xs text-stone-500">
            Only recipes estimated at {capLabel(cap)} or less. Estimates are a
            lower bound, and recipes we can't time yet still show.
          </p>
        {/if}
      {:else if cap !== null}
        <!-- Guests see the bound they will be swiping within — shown, not
             settable: the cap is the host's call (#80). -->
        <p class="mt-6 text-sm text-stone-600">
          <span
            class="rounded-pill mr-1 inline-flex items-center gap-2 px-3 py-1 text-sm font-medium {chosenPill}"
          >
            {#if hostId}
              <span
                class="size-2 shrink-0 rounded-full {userColour(hostId)}"
                aria-hidden="true"
              ></span>
            {/if}
            {capLabel(cap)}</span
          >
          Recipes estimated over this are left out. Estimates are a lower bound,
          and recipes we can't time yet still show.
        </p>
      {/if}

      <p class="mt-6 mb-3 text-xs text-stone-500">Who's deciding</p>
      <ul class="flex flex-col gap-1.5">
        {#each voters as v (v.telegram_user_id)}
          <li><UserName user={v} /></li>
        {/each}
      </ul>

      {#if host && candidates.length}
        <p class="mt-8 mb-3 text-xs text-stone-500">In this kitchen</p>
        <ul class="flex flex-col gap-2">
          {#each candidates as c (c.telegram_user_id)}
            <li class="flex items-center justify-between">
              <UserName user={c} />
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

      <!-- The way out (#96). Quiet — stone, not cocoa — because it is the opposite
           of the thing this page is for, but present for everyone including the
           host: the roster is the number a recipe has to win over, so a person who
           cannot leave holds the plan hostage. -->
      <div class="mt-8 border-t border-stone-200 pt-4">
        <button
          type="button"
          onclick={() => onLeave?.()}
          class="rounded-pill border border-stone-300 px-3 py-1 text-sm text-stone-600"
        >
          Leave
        </button>
        {#if lastOneHere}
          <p class="mt-2 text-xs text-stone-500">
            You're the only one here, so leaving ends this plan.
          </p>
        {:else if successor}
          <p class="mt-2 text-xs text-stone-500">
            You started this, so leaving hands it to <UserName
              user={successor}
              inline
            />.
          </p>
        {/if}
      </div>
    {/if}
  </Panel>
</div>
