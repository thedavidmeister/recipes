<script lang="ts">
  import type { Match, PickStatus, RecipeCard } from "$lib/types";

  /**
   * The pick swipe view (#20) — an endless, shared swipe focused on **consensus**.
   *
   * A pick keeps serving cards until the group finds a recipe everyone says yes to
   * — a **match**, which is the pick. Matches surface **inline** the moment they
   * happen; there is no separate results screen and no plurality. Presentational
   * only: the page owns the socket, the deck (which refills endlessly), and the
   * cross-pollination. Every state is a Storybook story.
   */
  interface Props {
    status: PickStatus;
    /** The card at the top of this client's deck, if any. */
    card?: RecipeCard;
    /** Recipes everyone in the pick said yes to — the pick(s), newest first. */
    matches?: Match[];
    /** How many people are in this pick (distinct voters). */
    participants?: number;
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
    matches = [],
    participants = 1,
    error,
    shareUrl,
    copied = false,
    onVote,
    onShare,
  }: Props = $props();

  const meta = $derived(
    card ? [card.category, card.area].filter(Boolean).join(" · ") : "",
  );
  const cardMeta = (c: RecipeCard) =>
    [c.category, c.area].filter(Boolean).join(" · ");
</script>

<div class="pt-6">
  <header class="mb-6 flex items-center justify-between gap-4">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-pesto-500" aria-hidden="true"></span>
      Pick
    </p>
    {#if shareUrl}
      <button
        onclick={() => onShare?.()}
        class="rounded-pill font-display inline-flex items-center gap-2 border border-stone-200 bg-cream-50 px-4 py-2 text-sm font-medium text-stone-900 transition-colors hover:border-pesto-500"
      >
        {copied ? "Link copied" : "Invite"}
      </button>
    {/if}
  </header>

  {#if status === "error"}
    <div class="rounded-card border border-paprika-500/30 bg-paprika-100 p-6">
      <p class="font-display text-stone-900">The pick dropped.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Could not reach the room."}
      </p>
    </div>
  {:else if status === "connecting"}
    <div
      class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center"
    >
      <p class="font-display text-stone-900">Starting a pick…</p>
      <p class="mt-1 text-sm text-stone-600">Catching up on the votes so far.</p>
    </div>
  {:else}
    {#if status === "reconnecting"}
      <p
        class="rounded-pill mb-3 inline-flex items-center gap-2 bg-honey-100 px-3 py-1 text-sm text-stone-600"
      >
        <span class="size-2 rounded-full bg-honey-500" aria-hidden="true"></span>
        Reconnecting…
      </p>
    {/if}

    {#if matches.length}
      <!-- The pick: consensus. Everyone in the room said yes to these. -->
      <section class="rounded-card mb-5 border border-pesto-500 bg-pesto-100 p-5">
        <p class="font-display flex items-center gap-2 font-medium text-pesto-500">
          <span class="size-2.5 rounded-full bg-pesto-500" aria-hidden="true"
          ></span>
          {matches.length === 1
            ? "It's a match — everyone's in"
            : "Everyone's in on these"}
        </p>
        <ul class="mt-3 flex flex-col gap-3">
          {#each matches as m (`${m.card.source}:${m.card.id}`)}
            <li class="flex items-center gap-3">
              {#if m.card.image}
                <img
                  src={m.card.image}
                  alt={m.card.title}
                  class="rounded-card size-14 flex-none object-cover"
                  loading="lazy"
                />
              {/if}
              <div class="min-w-0">
                <p class="font-display truncate font-medium text-stone-900">
                  {m.card.title}
                </p>
                {#if cardMeta(m.card)}
                  <p class="truncate text-sm text-stone-500">{cardMeta(m.card)}</p>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if !card}
      <div
        class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center"
      >
        <p class="font-display text-stone-900">Finding more recipes…</p>
        <p class="mt-1 text-sm text-stone-600">
          A pick keeps going until everyone agrees — the next card is on its way.
        </p>
      </div>
    {:else}
      <article
        class="rounded-card overflow-hidden border border-stone-200 bg-cream-100"
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
          {#if meta}
            <p class="mt-1 text-sm text-stone-500">{meta}</p>
          {/if}
        </div>
      </article>

      <div class="mt-5 flex items-center justify-center gap-4">
        <button
          onclick={() => onVote?.(false)}
          class="rounded-pill font-display border border-stone-200 bg-cream-50 px-8 py-3 font-medium text-stone-600 transition-colors hover:border-stone-400"
        >
          Pass
        </button>
        <button
          onclick={() => onVote?.(true)}
          class="rounded-pill font-display bg-pesto-500 px-8 py-3 font-medium text-cream-50 transition-colors hover:bg-pesto-500/90"
        >
          Yes
        </button>
      </div>
    {/if}

    <footer class="mt-6 border-t border-stone-200 pt-4">
      <p class="text-sm text-stone-500">
        {participants} deciding · {matches.length
          ? "keep swiping for another match"
          : "swipe to find something everyone likes"}
      </p>
    </footer>
  {/if}
</div>
