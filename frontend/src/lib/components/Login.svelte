<script lang="ts">
  import type { LoginStatus } from "$lib/types";

  /**
   * The login screen. Auth is mandatory (#25), so this is the first thing a
   * visitor meets — search included, because since #29 a search is an ingest.
   *
   * It only ever *points at* the bot. There is no "start login" button, because
   * a browser-initiated login is what let an attacker send someone a link and
   * take their session: the redeeming capability sat with whoever started the
   * login, while the identity came from whoever tapped. The bot mints the secret
   * for the person who messages it and sends the link to their chat.
   *
   * State comes in as props and the page owns the session query, per the
   * project's Storybook convention.
   */
  interface Props {
    status: LoginStatus;
    /** `https://t.me/<bot>`. */
    link: string;
    error?: string;
  }

  let { status, link, error }: Props = $props();
</script>

<div class="mx-auto flex max-w-md flex-col items-center px-4 py-16 text-center">
  <h1 class="font-display text-4xl font-medium tracking-tight text-stone-900">recipes</h1>

  {#if status === "checking"}
    <p class="mt-6 text-stone-500">Checking your session…</p>
  {:else if status === "error"}
    <p class="mt-6 text-tomato-500">{error ?? "Something went wrong."}</p>
    <p class="mt-2 text-sm text-stone-500">
      The site can't reach its backend. Try again in a moment.
    </p>
  {:else}
    <p class="mt-2 text-stone-500">Sign in with Telegram to continue.</p>

    <a
      href={link}
      target="_blank"
      rel="noopener noreferrer"
      class="mt-6 w-full rounded-full bg-tomato-500 px-4 py-3 font-display font-semibold text-cream-50 transition hover:brightness-105"
    >
      Sign in with Telegram
    </a>

    <p class="mt-4 text-sm text-stone-500">
      Press <span class="font-medium">Start</span> in Telegram and the bot will send
      you a link back. Open it and you're in.
    </p>
    <p class="mt-3 text-xs text-stone-400">
      Open the bot's link on this device — it signs in the browser you open it
      in. A Telegram account is required to use this site.
    </p>
  {/if}
</div>
