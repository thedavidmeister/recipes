<script lang="ts">
  import Alert from "./Alert.svelte";
  import Notice from "./Notice.svelte";
  import Panel from "./Panel.svelte";
  import type { PickStatus, RecipeCard } from "$lib/types";

  /**
   * The pick swipe view (#20) — an endless, shared swipe for **consensus**.
   *
   * A pick keeps serving cards until everyone agrees on **one** recipe; the instant
   * that happens the page whisks everyone straight to `buy` (its ingredients), so
   * this view is purely the swipe. Presentational only: the page owns the socket,
   * the deck (which refills endlessly), and the cross-pollination. Every state is a
   * Storybook story.
   */
  interface Props {
    status: PickStatus;
    /** The card at the top of this client's deck, if any. */
    card?: RecipeCard;
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
</script>

<div class="pt-32 pb-16">
  <Panel>
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
    <Alert>
      <p class="font-display text-stone-900">The pick dropped.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Could not reach the room."}
      </p>
    </Alert>
  {:else if status === "connecting"}
    <Notice>
      <p class="font-display text-stone-900">Starting a pick…</p>
      <p class="mt-1 text-sm text-stone-600">Catching up on the votes so far.</p>
    </Notice>
  {:else}
    {#if status === "reconnecting"}
      <p
        class="rounded-pill mb-3 inline-flex items-center gap-2 bg-honey-100 px-3 py-1 text-sm text-stone-600"
      >
        <span class="size-2 rounded-full bg-honey-500" aria-hidden="true"></span>
        Reconnecting…
      </p>
    {/if}

    {#if !card}
      <Notice>
        <p class="font-display text-stone-900">Finding more recipes…</p>
        <p class="mt-1 text-sm text-stone-600">
          A pick keeps going until everyone agrees — the next card is on its way.
        </p>
      </Notice>
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
        {participants} deciding · swipe to find something everyone likes
      </p>
    </footer>
  {/if}
</Panel>
</div>
