<script lang="ts">
  import Skeleton from "./Skeleton.svelte";
  import Panel from "./Panel.svelte";
  import Button from "./Button.svelte";
  import type { Voter } from "$lib/pick";
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
    /** The shareable URL that seats whoever opens it. */
    inviteLink?: string;
    /** Whether the viewer is the one who started the plan. */
    host?: boolean;
    error?: string;
    onStart?: () => void;
    /** Add a kitchen member by id. Host only. */
    onSeat?: (userId: string) => void;
  }

  let {
    status,
    voters = [],
    candidates = [],
    inviteLink,
    host = false,
    error,
    onStart,
    onSeat,
  }: Props = $props();

  const name = (v: Voter) => (v.username ? `@${v.username}` : v.telegram_user_id);

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
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="bg-pesto-500 size-2.5 rounded-full" aria-hidden="true"></span>
      Meal plan
    </p>

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
