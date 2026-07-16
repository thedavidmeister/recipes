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
  <h1 class="font-display text-4xl font-medium tracking-tight text-stone-900">
    recipes
  </h1>

  {#if status === "checking"}
    <p class="mt-6 text-stone-500">Checking your session…</p>
  {:else if status === "error"}
    <p class="text-paprika-500 mt-6">{error ?? "Something went wrong."}</p>
    <p class="mt-2 text-sm text-stone-500">
      The site can't reach its backend. Try again in a moment.
    </p>
  {:else}
    <p class="mt-2 text-stone-500">Sign in with Telegram to continue.</p>

    <a
      href={link}
      target="_blank"
      rel="noopener noreferrer"
      class="bg-cream-50 font-display mt-6 flex w-full items-center justify-center gap-2 rounded-xl border-2 border-stone-300 px-4 py-3 font-semibold text-stone-900 transition hover:border-stone-400"
    >
      <span class="bg-pesto-500 size-2.5 rounded-full"></span>
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
