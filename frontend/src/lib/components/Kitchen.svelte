<script lang="ts">
  import type { KitchenDetail, KitchensStatus } from "$lib/types";
  import QrCode from "./QrCode.svelte";

  /**
   * One kitchen (#72): who is in it and how to invite someone. Its equipment, its
   * pantry and its name are their own pages — this only links to them, so the page
   * stays one idea.
   *
   * A primary kitchen says so. It is the one made for you and the one the app assumes,
   * so the difference between it and a kitchen you opened on purpose is worth naming.
   */
  interface Props {
    status: KitchensStatus;
    kitchen?: KitchenDetail | null;
    /** The shareable invite URL (the page builds it from the token). */
    inviteLink?: string;
    error?: string;
    /** Start a meal plan in this kitchen — the lobby is the next page. */
    onPlan?: () => void;
  }

  let { status, kitchen, inviteLink, error, onPlan }: Props = $props();

  let copied = $state(false);

  async function copyInvite() {
    if (!inviteLink) return;
    try {
      await navigator.clipboard.writeText(inviteLink);
      copied = true;
    } catch {
      // Clipboard blocked — the link is there to copy by hand.
    }
  }
</script>

<div class="pt-48 pb-16">
  <div class="rounded-card bg-cream-50 p-6">
    <a href="/kitchens" class="text-sm text-stone-500 underline">← Kitchens</a>

    {#if status === "error" || (status === "ready" && !kitchen)}
      <p class="mt-4 text-sm text-stone-600">
        {error ?? "Couldn't open this kitchen."}
      </p>
    {:else if status === "pending" || !kitchen}
      <div class="rounded-card mt-4 h-10 w-full bg-stone-100" aria-hidden="true"></div>
    {:else}
      <div class="mt-3 flex items-center gap-3">
        <h1 class="font-display text-2xl font-medium text-stone-900">{kitchen.name}</h1>
        <span
          class="rounded-pill flex-none border border-cocoa-500 px-2.5 py-0.5 text-xs text-cocoa-500"
        >
          {kitchen.role}
        </span>
      </div>
      {#if kitchen.is_primary}
        <p class="mt-1 text-xs text-stone-500">
          Yours by default — the kitchen the app works in until you open another.
        </p>
      {/if}

      <button
        type="button"
        onclick={onPlan}
        class="rounded-card font-display bg-cocoa-500 text-cream-50 mt-5 flex w-full items-center justify-between px-4 py-3"
      >
        Plan a meal here
        <span class="text-cream-200 text-sm">→</span>
      </button>

      <ul class="mt-5 flex flex-col gap-2">
        {#if kitchen.role === "owner"}
          <li>
            <a
              href="/kitchens/{kitchen.id}/name"
              class="rounded-card font-display flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
            >
              Rename
              <span class="text-sm text-stone-400">→</span>
            </a>
          </li>
        {/if}
        <li>
          <a
            href="/kitchens/{kitchen.id}/equipment"
            class="rounded-card font-display flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
          >
            Equipment
            <span class="text-sm text-stone-400">{kitchen.equipment.length} →</span>
          </a>
        </li>
        <li>
          <a
            href="/kitchens/{kitchen.id}/pantry"
            class="rounded-card font-display flex items-center justify-between border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
          >
            Pantry
            <span class="text-sm text-stone-400">{kitchen.pantry.length} →</span>
          </a>
        </li>
      </ul>

      <p class="mt-8 mb-3 text-xs text-stone-500">Who's in it</p>
      <ul class="flex flex-col gap-1.5">
        {#each kitchen.members as m (m.telegram_user_id)}
          <li class="flex items-baseline gap-2 text-stone-900">
            <span class="font-display">
              {m.username ? `@${m.username}` : m.telegram_user_id}
            </span>
            <span class="text-sm text-stone-400">— {m.role}</span>
          </li>
        {/each}
      </ul>

      {#if inviteLink}
        <p class="mt-8 mb-3 text-xs text-stone-500">Invite someone</p>
        <div class="flex flex-col items-center gap-3">
          <QrCode value={inviteLink} label="Scan to join {kitchen.name}" />
          <button
            type="button"
            onclick={copyInvite}
            class="rounded-pill border border-cocoa-500 px-3 py-1 text-sm text-cocoa-500"
          >
            {copied ? "Copied" : "Copy invite link"}
          </button>
        </div>
      {/if}
    {/if}
  </div>
</div>
