<script lang="ts">
  import type { LoginStatus } from "$lib/types";

  /**
   * The login screen. Auth is mandatory (#25), so this is the first thing a
   * visitor meets — search included, because since #29 a search is an ingest.
   *
   * State comes in as props and the page owns the polling, per the project's
   * Storybook convention: `waiting` and `expired` are impossible to click to
   * reliably (one needs a stranger to tap a link, the other needs 15 minutes),
   * so they must be declarable.
   */
  interface Props {
    status: LoginStatus;
    /** The `t.me` deep link. Present while `waiting`. */
    link?: string;
    error?: string;
    onStart?: () => void;
  }

  let { status, link, error, onStart }: Props = $props();
</script>

<div class="mx-auto flex max-w-md flex-col items-center px-4 py-16 text-center">
  <h1 class="text-3xl font-bold tracking-tight">recipes</h1>

  {#if status === "checking"}
    <p class="mt-6 text-neutral-500">Checking your session…</p>
  {:else if status === "waiting" && link}
    <p class="mt-2 text-neutral-500">Sign in with Telegram to continue.</p>

    <!--
      The link goes TO the bot. A bot cannot message someone who has not
      contacted it first, so there is no "we'll DM you a link" — tapping this is
      what introduces you, and pressing Start is what signs you in.
    -->
    <a
      href={link}
      target="_blank"
      rel="noopener noreferrer"
      class="mt-6 w-full rounded-lg bg-sky-600 px-4 py-3 font-medium text-white hover:bg-sky-500"
    >
      Open Telegram
    </a>

    <p class="mt-4 text-sm text-neutral-500">
      Waiting for you to press <span class="font-medium">Start</span> in
      Telegram. This page will continue on its own.
    </p>
    <p class="mt-2 text-xs text-neutral-400">
      The link expires in 15 minutes. It only signs in the account that taps it.
    </p>
  {:else if status === "starting"}
    <p class="mt-6 text-neutral-500">Preparing your login link…</p>
  {:else if status === "expired"}
    <p class="mt-6 text-neutral-600">That login link expired.</p>
    <button
      onclick={onStart}
      class="mt-4 rounded-lg bg-neutral-900 px-4 py-2 font-medium text-white hover:bg-neutral-700"
    >
      Get a new link
    </button>
  {:else if status === "error"}
    <p class="mt-6 text-red-700">{error ?? "Something went wrong."}</p>
    <button
      onclick={onStart}
      class="mt-4 rounded-lg bg-neutral-900 px-4 py-2 font-medium text-white hover:bg-neutral-700"
    >
      Try again
    </button>
  {:else}
    <p class="mt-2 text-neutral-500">Sign in with Telegram to continue.</p>
    <button
      onclick={onStart}
      class="mt-6 w-full rounded-lg bg-sky-600 px-4 py-3 font-medium text-white hover:bg-sky-500"
    >
      Sign in with Telegram
    </button>
    <p class="mt-4 text-xs text-neutral-400">
      A Telegram account is required to use this site.
    </p>
  {/if}
</div>
