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
   */
  interface Props {
    status: "pending" | "error" | "ready";
    /** The shareable URL that seats whoever opens it. */
    link?: string;
    kitchen?: string;
    error?: string;
  }

  let { status, link, kitchen, error }: Props = $props();

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
    {:else if status === "pending" || !link}
      <div
        class="rounded-card mt-4 h-10 w-full bg-stone-100"
        aria-hidden="true"
      ></div>
    {:else}
      <p class="mt-4 text-sm text-stone-600">
        Anyone who opens this joins {kitchen ?? "the kitchen"} — the same as everyone
        else in it.
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
