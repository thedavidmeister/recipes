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
   * long there is are all the host's calls, and a chosen pill wears the host's
   * colour whole — their tint, their edge, their dot (#131). A guest reads "someone
   * decided this, and it was them" off the pill itself, not off a flat grey fill
   * that could have come from anywhere.
   *
   * A resting pill is in the app's ordinary button language — cream, a quiet stone
   * outline, secondary ink — which is the half of #164 that was right: a cocoa
   * outline *and* cocoa text made the meal row a different species of control from
   * every other button on the page. The half that was wrong was answering that by
   * draining the colour out of the chosen ones too, which only moved the brown from
   * the outline into the fill.
   *
   * Chosen-ness does not lean on colour perception: `aria-pressed`, the full-strength
   * label and the dot all say it, and the tint is the fourth signal rather than the
   * only one.
   */
  interface Props {
    /**
     * `ended` is the plan everybody walked out of (#96). It belongs here rather
     * than on the swipe view because a plan can only be emptied *before* it starts
     * — the roster closes at the start in both directions — so the lobby is the
     * only screen that can still be open on a plan that no longer exists.
     */
    status: "pending" | "error" | "ready" | "ended";
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
     * cannot leave is trapped in their own plan.
     *
     * And **only** here. The roster closes at the start in both directions, so the
     * lobby is the whole of the way out: once the swiping begins the set of people
     * a recipe has to win over is fixed, and neither a late joiner nor a departure
     * may move it under everybody. */
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

  // "with dessert & side" — additions as a quiet prose list.
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
   * The classes for a pill the host has chosen: **the host's own colour**, all the
   * way through — their `-100` tint as the fill, their `-500` as the edge, their dot
   * beside the label, ink at full strength.
   *
   * A choice in this lobby is somebody's, and this app says whose in colour (#131).
   * The one thing the edge may not be is brown: a cocoa or stone outline drawn round
   * a fill in somebody's hue reads as two decisions arguing, which is the whole of
   * what #164 was about. The edge belongs to whoever the fill belongs to.
   *
   * It is reinforcement rather than the boundary carrying the meaning — the pill is
   * identified by its `stone-900` label, its dot and `aria-pressed`, so nothing here
   * asks a `-500` to be a control's boundary on its own. That is what lets the two
   * palest slots wear their own edge like the other four, and none of the six is
   * weaker than the neutral it replaces: `honey-500`, the palest, sits at 2.0:1 on
   * cream against the `stone-300` unchosen border's 1.65:1.
   *
   * With no host loaded yet there is no colour to wear, so the pill falls back to the
   * resting outline and simply has no dot — an unattributed choice, not a wrong one.
   */
  const chosenPill = (id: string | undefined) =>
    id
      ? `border ${userEdge(id)} ${userTint(id)} text-stone-900`
      : "border border-stone-300 text-stone-900";

  /**
   * A pill nobody has chosen: the resting `Button` language exactly — cream paper, a
   * quiet `stone-300` outline, secondary ink. It was a cocoa outline **and** cocoa
   * text, which made the meal row look like a different species of control from every
   * other button on the page (#164 again, the half of it that was right).
   */
  const RESTING_PILL = "border border-stone-300 text-stone-600";

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
           and the side come with it. -->
      <p class="mt-1 text-sm text-stone-500">
        with {listAdditions(additions)}
      </p>
    {/if}

    {#if status === "ended"}
      <!-- Stated, not styled as a failure: nothing went wrong. The last person in
           the plan left it, so it is simply over (#96) and the link no longer
           resolves. Said plainly, in the same place the error would be, because
           the one thing this screen must not do is keep offering a Start and a
           Leave for a plan that is gone. -->
      <p class="font-display mt-4 text-stone-900">This meal plan is over.</p>
      <p class="mt-1 text-sm text-stone-600">
        Everyone left it, so there is nothing left to decide.
      </p>
    {:else if status === "error"}
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
                ? chosenPill(hostId)
                : RESTING_PILL}"
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
             row because they are not what the pick decides. -->
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
                ? chosenPill(hostId)
                : RESTING_PILL}"
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
                ? chosenPill(hostId)
                : RESTING_PILL}"
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
            Only recipes that fit in {capLabel(cap)}. A time is the least a
            recipe can take, so some run longer — and ones with no time on them
            still show.
          </p>
        {/if}
      {:else if cap !== null}
        <!-- Guests see the bound they will be swiping within — shown, not
             settable: the cap is the host's call (#80). -->
        <p class="mt-6 text-sm text-stone-600">
          <span
            class="rounded-pill mr-1 inline-flex items-center gap-2 px-3 py-1 text-sm font-medium {chosenPill(hostId)}"
          >
            {#if hostId}
              <span
                class="size-2 shrink-0 rounded-full {userColour(hostId)}"
                aria-hidden="true"
              ></span>
            {/if}
            {capLabel(cap)}</span
          >
          Anything longer is left out. A time is the least a recipe can take, so
          some run longer — and ones with no time on them still show.
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
