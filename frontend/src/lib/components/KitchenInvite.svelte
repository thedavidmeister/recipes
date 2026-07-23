<script lang="ts">
  import QrCode from "./QrCode.svelte";

  /**
   * Inviting someone into a kitchen (#72) — its own page, because it is a thing you
   * are doing rather than a corner of the kitchen you are looking at, and because the
   * code wants room. Handing someone a phone to scan is the whole interaction; it
   * should not be the last item on a long page.
   *
   * Whoever opens the link joins as an owner, like everyone else in the room. There is
   * no lesser membership to offer them — being a guest is something you are at a meal.
   *
   * Which is exactly why the link expires, and why the page says so. Opening this mints
   * a fresh invite good for two hours: full access handed out by a link that never died
   * would mean a screenshot is a permanent key to somebody's kitchen.
   */
  interface Props {
    status: "pending" | "error" | "ready";
    /** The shareable URL that seats whoever opens it. */
    link?: string;
    kitchen?: string;
    /**
     * Seconds until the link dies, counted down by the page.
     *
     * The clock is not read here on purpose: a component that ticks by itself renders
     * differently every time it is rendered, which the visual fence would catch as a
     * change on every run and which no story could pin. The page owns the ticking, the
     * same way it owns the query.
     */
    remaining?: number;
    error?: string;
    /** Mint a fresh link, for when this one has run out. */
    onRenew?: () => void;
  }

  let { status, link, kitchen, remaining, error, onRenew }: Props = $props();

  const dead = $derived(remaining !== undefined && remaining <= 0);

  /** Coarse near the top, exact near the end — the last minute is when it matters. */
  const countdown = $derived.by(() => {
    if (remaining === undefined) return undefined;
    if (remaining <= 0) return undefined;
    if (remaining < 60) return `${remaining}s`;
    const mins = Math.floor(remaining / 60);
    const hours = Math.floor(mins / 60);
    return hours > 0 ? `${hours}h ${mins % 60}m` : `${mins}m`;
  });

  let copied = $state(false);

  async function copy() {
    if (!link) return;
    try {
      await navigator.clipboard.writeText(link);
      copied = true;
    } catch {
      // Clipboard blocked — the link is on screen to copy by hand.
    }
  }
</script>

<div class="pt-48 pb-16">
  <div class="rounded-card bg-cream-50 p-6">
    <p class="font-display flex items-center gap-2 text-stone-600">
      <a href="/kitchens" class="text-stone-500 underline">Kitchens</a>
      <span aria-hidden="true">·</span>
      Invite
    </p>

    {#if status === "error"}
      <p class="mt-4 text-sm text-stone-600">
        {error ?? "Couldn't open this kitchen."}
      </p>
    {:else if dead}
      <p class="mt-4 text-sm text-stone-600">
        This link has run out. Links last two hours, so one that has been sitting open
        is no use to anybody — including whoever finds it.
      </p>
      <button
        type="button"
        onclick={onRenew}
        class="rounded-card bg-cocoa-500 text-cream-50 mt-6 w-full px-4 py-3 font-medium"
      >
        Make a new link
      </button>
    {:else if status === "pending" || !link}
      <div
        class="rounded-card mt-4 h-10 w-full bg-stone-100"
        aria-hidden="true"
      ></div>
    {:else}
        <p class="mt-4 text-sm text-stone-600">
          Anyone who opens this joins {kitchen ?? "the kitchen"} — the same as
          everyone else in it.
        </p>
        <p class="mt-1 text-sm text-stone-500">
          {countdown
            ? `Good for ${countdown}, then it stops working.`
            : "Good for two hours, then it stops working."}
        </p>

        <div class="mt-8 flex flex-col items-center gap-5">
          <QrCode value={link} label="Scan to join {kitchen ?? 'this kitchen'}" />
          <button
            type="button"
            onclick={copy}
            class="rounded-pill border-cocoa-500 text-cocoa-500 border px-4 py-2 text-sm font-medium"
          >
            {copied ? "Copied" : "Copy invite link"}
          </button>
        </div>
    {/if}
  </div>
</div>
