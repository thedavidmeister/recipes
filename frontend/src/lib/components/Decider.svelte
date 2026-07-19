<script lang="ts">
  import type { RecipeCard, SessionStatus } from "$lib/types";

  /**
   * The cook-decider swipe view (#20) — the multiplayer mode of `pick`.
   *
   * One card at a time: yes keeps it, pass drops it, and the tally is the group's
   * running decision. Presentational only — the page owns the WS client, the deck,
   * and the silent peer-injection (a recipe anyone votes on appears in everyone's
   * deck). Every state — joining, reconnecting, a card to swipe, all caught up, a
   * dropped room — is a Storybook story, not something you race the socket to reach.
   */
  interface Props {
    status: SessionStatus;
    /** The card at the top of this client's deck, if any. */
    card?: RecipeCard;
    /** Recipes with at least one yes so far — the size of "the running". */
    inTheRunning?: number;
    /** How many people are deciding (distinct voters). */
    participants?: number;
    error?: string;
    /** The shareable link that invites others into this session. */
    shareUrl?: string;
    /** `true` right after the link was copied — flips the Invite button's label. */
    copied?: boolean;
    onVote?: (yes: boolean) => void;
    onShare?: () => void;
    onWinners?: () => void;
  }

  let {
    status,
    card,
    inTheRunning = 0,
    participants = 1,
    error,
    shareUrl,
    copied = false,
    onVote,
    onShare,
    onWinners,
  }: Props = $props();

  const meta = $derived(
    card ? [card.category, card.area].filter(Boolean).join(" · ") : "",
  );
</script>

<div class="pt-6">
  <header class="mb-6 flex items-center justify-between gap-4">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-pesto-500" aria-hidden="true"></span>
      Decide together
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
      <p class="font-display text-stone-900">The session dropped.</p>
      <p class="mt-1 text-sm text-stone-600">
        {error ?? "Could not reach the room."}
      </p>
    </div>
  {:else if status === "connecting"}
    <div
      class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center"
    >
      <p class="font-display text-stone-900">Joining the session…</p>
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

    {#if status === "empty" || !card}
      <div
        class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center"
      >
        <p class="font-display text-stone-900">You're all caught up.</p>
        <p class="mt-1 text-sm text-stone-600">
          Waiting for the others — anything they vote on lands in your deck.
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

    <footer
      class="mt-6 flex items-center justify-between gap-4 border-t border-stone-200 pt-4"
    >
      <p class="text-sm text-stone-500">
        {inTheRunning} in the running · {participants} deciding
      </p>
      <button
        onclick={() => onWinners?.()}
        disabled={inTheRunning === 0}
        class="rounded-pill font-display border border-stone-200 bg-cream-50 px-4 py-2 text-sm font-medium text-stone-900 transition-colors hover:border-pesto-500 disabled:opacity-50"
      >
        See winners
      </button>
    </footer>
  {/if}
</div>
