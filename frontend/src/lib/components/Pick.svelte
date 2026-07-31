<script lang="ts">
  import Alert from "./Alert.svelte";
  import Notice from "./Notice.svelte";
  import Panel from "./Panel.svelte";
  import UserName from "./UserName.svelte";
  import { userTint } from "$lib/colour";
  import { waitingOnOthers } from "$lib/deal";
  import { formatEstimate } from "$lib/steps";
  import type { Voter } from "$lib/pick";
  import type { PickStatus, RecipeCard } from "$lib/types";

  /**
   * The pick swipe view (#20) — an endless, shared swipe for **consensus**.
   *
   * A pick keeps serving cards until everyone agrees on **one** recipe; the instant
   * that happens the page whisks everyone straight to `buy` (its ingredients), so
   * this view is purely the swipe. Presentational only: the page owns the socket,
   * the deck (which refills endlessly), and the cross-pollination. Every state is a
   * Storybook story.
   *
   * A card mostly gets here *because* somebody voted it — that is what
   * cross-pollination is — so the people who already said yes are named under it,
   * each in their own colour (#131/#145). It turns a solitary sort into the
   * conversation it actually is: you are not rating recipes, you are answering Mel.
   */
  interface Props {
    status: PickStatus;
    /** The card at the top of this client's deck, if any. */
    card?: RecipeCard;
    /** How many a recipe has to win over — the plan's roster, not who has swiped so
     * far (#181). One when you are the only one in the plan. */
    participants?: number;
    /** Who has already said yes to this card. */
    yesVoters?: Voter[];
    /**
     * Whether this person has answered every recipe the plan can currently deal them
     * (#202), so an empty deck is **finished** rather than loading.
     *
     * A prop of its own rather than another [`PickStatus`], because it is not a phase of
     * the connection like the others — it is a fact about the deal, and it is true or
     * false independently of every one of them. As a status it would have had to lose a
     * race with `reconnecting`, which would put "Finding more recipes…" back in front of
     * exactly the person this state exists for.
     */
    finished?: boolean;
    error?: string;
    /** The shareable link that invites others into this pick. */
    shareUrl?: string;
    copied?: boolean;
    onVote?: (yes: boolean) => void;
    onShare?: () => void;
  }

  let {
    status,
    card,
    participants = 1,
    yesVoters = [],
    finished = false,
    error,
    shareUrl,
    copied = false,
    onVote,
    onShare,
  }: Props = $props();

  const meta = $derived(
    card ? [card.category, card.area].filter(Boolean).join(" · ") : "",
  );

  /**
   * "How long does this take" as a badge, so it factors into the yes/no at a glance
   * instead of surfacing after the pick (#84). Null for a recipe nobody has timed:
   * that is unknown, not instant, so the badge simply is not there.
   *
   * The card's own `fully_timed` picks the mark (#158): `~23 min` when every step of
   * it carries a duration, `23 min+` when one does not and the total can therefore
   * only be too low. Per card, not per deploy — the corpus is re-read a recipe at a
   * time, so two cards in one deck can honestly wear different marks.
   */
  const estimate = $derived(
    card ? formatEstimate(card.total_seconds, card.fully_timed) : null,
  );

  /**
   * What the badge's mark means, on hover. It has to follow `fully_timed` for the
   * same reason the mark does: a `~19 min` card carries no untimed steps, so telling
   * its reader the number is only a floor is simply wrong about that card. One
   * tooltip for both marks said the `+` sentence over every `~`.
   */
  const estimateHint = $derived(
    card?.fully_timed
      ? "Roughly how long it takes, start to finish."
      : "At least this long — some steps here have no time on them, so the cooking runs longer.",
  );
</script>

<div class="pt-32 pb-16">
  <Panel>
    <header class="mb-6 flex items-center justify-between gap-4">
      <p class="font-display flex items-center gap-2 text-stone-600">
        <span class="bg-pesto-500 size-2.5 rounded-full" aria-hidden="true"
        ></span>
        Pick
      </p>
      {#if shareUrl}
        <button
          onclick={() => onShare?.()}
          class="rounded-pill font-display bg-cream-50 hover:border-pesto-500 inline-flex items-center gap-2 border border-stone-200 px-4 py-2 text-sm font-medium text-stone-900 transition-colors"
        >
          {copied ? "Link copied" : "Invite"}
        </button>
      {/if}
    </header>

    {#if status === "error"}
      <Alert>
        <p class="font-display text-stone-900">Lost the connection.</p>
        <p class="mt-1 text-sm text-stone-600">
          {error ?? "Reload the page to rejoin the others."}
        </p>
      </Alert>
    {:else if status === "connecting"}
      <Notice>
        <p class="font-display text-stone-900">Starting a pick…</p>
        <p class="mt-1 text-sm text-stone-600">
          Catching up on the votes so far.
        </p>
      </Notice>
    {:else}
      {#if status === "reconnecting"}
        <p
          class="rounded-pill bg-honey-100 mb-3 inline-flex items-center gap-2 px-3 py-1 text-sm text-stone-600"
        >
          <span class="bg-honey-500 size-2 rounded-full" aria-hidden="true"
          ></span>
          Reconnecting…
        </p>
      {/if}

      {#if !card && finished}
        <!-- An empty deck, for the other reason: everything this plan can deal you has
             been answered (see the `finished` prop). Waiting on the others is a real
             state and it says so, rather than hunting for a card that is not coming.
             Not an error and nothing to do, so it wears the same quiet Notice the other
             empty states wear, with the roster it is waiting on named under it the way
             the footer already names it. -->
        <Notice>
          <p class="font-display text-stone-900">You've answered everything.</p>
          <p class="mt-1 text-sm text-stone-600">
            {waitingOnOthers(participants)}
          </p>
        </Notice>
      {:else if !card}
        <Notice>
          <p class="font-display text-stone-900">Finding more recipes…</p>
          <p class="mt-1 text-sm text-stone-600">
            A pick keeps going until everyone agrees — the next card is on its
            way.
          </p>
        </Notice>
      {:else}
        <article
          class="rounded-card bg-cream-100 overflow-hidden border border-stone-200"
        >
          {#if card.image}
            <img
              src={card.image}
              alt={card.title}
              class="rounded-card aspect-video w-full object-cover"
              loading="lazy"
            />
          {/if}
          <div class="p-5">
            <h2 class="font-display text-xl font-medium text-stone-900">
              {card.title}
            </h2>
            <!-- Supporting information, not the headline: the meal is still the
               title and the photo. The estimate sits beside the category/area on
               the same quiet line, and is absent entirely when unknown. -->
            {#if meta || estimate}
              <p
                class="mt-1 flex flex-wrap items-center gap-2 text-sm text-stone-500"
              >
                {#if meta}<span>{meta}</span>{/if}
                {#if estimate}
                  <span
                    class="rounded-pill bg-cream-200 px-2 py-0.5 text-xs text-stone-600"
                    title={estimateHint}>{estimate}</span
                  >
                {/if}
              </p>
            {/if}
          </div>
        </article>

        {#if yesVoters.length}
          <!-- Whose yes this already is. The tint carries the person and the name
             carries the meaning: the colour is never the only signal, which is what
             lets all six slots be used, pale ones included. -->
          <p class="mt-4 text-xs text-stone-500">Already a yes for</p>
          <ul class="mt-2 flex flex-wrap gap-2">
            {#each yesVoters as v (v.telegram_user_id)}
              <li
                class="rounded-pill px-3 py-1 text-sm {userTint(
                  v.telegram_user_id,
                )}"
              >
                <UserName user={v} />
              </li>
            {/each}
          </ul>
        {/if}

        <div class="mt-5 flex items-center justify-center gap-4">
          <button
            onclick={() => onVote?.(false)}
            class="rounded-pill font-display bg-cream-50 border border-stone-200 px-8 py-3 font-medium text-stone-600 transition-colors hover:border-stone-400"
          >
            Pass
          </button>
          <button
            onclick={() => onVote?.(true)}
            class="rounded-pill font-display bg-pesto-500 text-cream-50 hover:bg-pesto-500/90 px-8 py-3 font-medium transition-colors"
          >
            Yes
          </button>
        </div>
      {/if}

      <footer class="mt-6 border-t border-stone-200 pt-4">
        <p class="text-sm text-stone-500">
          {participants} deciding · swipe to find something everyone likes
        </p>
      </footer>
    {/if}
  </Panel>
</div>
