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
    /** `https://t.me/<bot>`, or the same with a `?start=` payload carrying the
     * page to come back to (#206). */
    link: string;
    /**
     * Whether `link` carries somewhere to come back to — a plan someone scanned an
     * invite to, most of the time. It changes only what the screen *says*: the
     * whole of a login is still messaging the bot, and this promise is worth making
     * only where it is true, so a destination too long for Telegram to carry says
     * nothing rather than something optimistic.
     */
    returning?: boolean;
    error?: string;
  }

  let { status, link, returning = false, error }: Props = $props();
</script>

<div class="mx-auto flex max-w-md flex-col items-center px-4 py-16 text-center">
  <h1 class="font-display text-4xl font-medium tracking-tight text-stone-900">
    recipes
  </h1>

  {#if status === "checking"}
    <p class="mt-6 text-stone-500">Checking if you're signed in…</p>
  {:else if status === "error"}
    <p class="text-paprika-500 mt-6">
      {error ?? "Couldn't check whether you're signed in."}
    </p>
    <p class="mt-2 text-sm text-stone-500">
      The site can't be reached right now. Try again in a moment.
    </p>
  {:else}
    <!-- Reached from an invite, this screen is in the way of somewhere: say that
         signing in goes there, so the trip through Telegram reads as the way to
         the plan rather than a detour away from it. -->
    <p class="mt-2 text-stone-500">
      {returning
        ? "Sign in with Telegram and you'll come back to this page."
        : "Sign in with Telegram to continue."}
    </p>

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
      Press <span class="font-medium">Start</span> in Telegram — or send
      <span class="font-medium">/login</span> if you've messaged the bot before —
      and it will send you a link back. Open it and you're in.
    </p>
    <p class="mt-3 text-xs text-stone-400">
      Open the bot's link on this device — it signs in the browser you open it
      in. A Telegram account is required to use this site.
    </p>
  {/if}
</div>
