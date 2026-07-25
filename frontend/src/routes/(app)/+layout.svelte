<script lang="ts">
  import { page } from "$app/state";
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { me, logout, botLink } from "$lib/auth";
  import type { LoginStatus, Section } from "$lib/types";
  import { loginStatus } from "$lib/resource";
  import Login from "$lib/components/Login.svelte";
  import Nav from "$lib/components/Nav.svelte";
  import MusicSwitch from "$lib/components/MusicSwitch.svelte";
  import KitchenBackdrop from "$lib/components/KitchenBackdrop.svelte";
  import PickBackdrop from "$lib/components/PickBackdrop.svelte";
  import { pageSlide } from "$lib/transition";

  let { children } = $props();
  const queryClient = useQueryClient();

  const MUSIC_PREFERENCE = "recipes:music";
  /** Where a fade settles. A constant, not a setting — see MusicSwitch. */
  const LEVEL = 0.5;
  /** How long a track takes to cross to another, in milliseconds. */
  const FADE = 2500;

  /**
   * The music (#88, #121), now **per section**: kitchens has its bed, pick has its
   * own, and the rest of the app is quiet. Moving between them does not cut — one
   * track fades down as the next fades up, so a navigation feels like walking from
   * one room into another rather than a hard edit.
   *
   * A crossfade needs two things playing at once, so there are two voices. Whichever
   * is audible fades out and pauses; the other loads the new section's track and fades
   * in. A section with no track is silence, reached the same way — the audible voice
   * just fades to nothing.
   */
  function trackFor(pathname: string): string | null {
    const section = pathname.split("/")[1];
    if (section === "kitchens") return "/kitchen.mp3";
    if (section === "pick") return "/pick.mp3";
    return null; // the rest of the app is quiet, for now
  }

  const wanted = $derived(trackFor(page.url.pathname));

  /** Is the music switched on. Persisted, and the switch reflects it. */
  let on = $state(false);
  let a: HTMLAudioElement | undefined = $state();
  let b: HTMLAudioElement | undefined = $state();

  // The voice currently carrying `playingSrc`, and the track it is playing. Null when
  // silent — either the section has no track or the music is off.
  let live: HTMLAudioElement | undefined;
  let playingSrc: string | null = null;

  const fades = new Map<HTMLAudioElement, number>();
  function fadeTo(el: HTMLAudioElement, target: number, done?: () => void) {
    const prev = fades.get(el);
    if (prev !== undefined) cancelAnimationFrame(prev);
    const from = el.volume;
    const startedAt = performance.now();
    const step = () => {
      const through = Math.min(1, (performance.now() - startedAt) / FADE);
      el.volume = from + (target - from) * through;
      if (through < 1) {
        fades.set(el, requestAnimationFrame(step));
      } else {
        fades.delete(el);
        done?.();
      }
    };
    fades.set(el, requestAnimationFrame(step));
  }

  /**
   * Make what is playing match the section — and the on/off preference.
   *
   * Idempotent: if the right track is already the live one, it does nothing, which is
   * why it is safe to call on every navigation, on the on/off toggle, and from the
   * gesture listeners below. The browser only grants audio to a real user gesture (or
   * an origin it has learned you play audio on), so `play()` may be refused early;
   * when it is, `live` stays null and the next gesture tries again.
   */
  function applyMusic() {
    const voices = [a, b].filter((v): v is HTMLAudioElement => !!v);
    if (voices.length < 2) return;

    const want = on ? wanted : null;
    if (want === playingSrc) return;

    if (live) {
      const old = live;
      fadeTo(old, 0, () => old.pause());
    }
    if (!want) {
      live = undefined;
      playingSrc = null;
      return;
    }
    const next = voices.find((v) => v !== live) ?? voices[0];
    if (next.getAttribute("src") !== want) next.src = want;
    next.volume = 0;
    next.play().then(
      () => {
        fadeTo(next, LEVEL);
        live = next;
        playingSrc = want;
      },
      () => {
        // Refused: no gesture credited yet. A gesture listener will retry.
      },
    );
  }

  // React to the section changing and to the on/off toggle.
  $effect(() => {
    void wanted;
    void on;
    applyMusic();
  });

  // Start `on` from the remembered preference, and retry on any interaction until the
  // browser lets the audio through — the same gesture rule as before, now driving the
  // crossfade rather than a single element.
  $effect(() => {
    on = localStorage.getItem(MUSIC_PREFERENCE) !== "off";
    const retry = () => applyMusic();
    window.addEventListener("pointerdown", retry);
    window.addEventListener("keydown", retry);
    return () => {
      window.removeEventListener("pointerdown", retry);
      window.removeEventListener("keydown", retry);
    };
  });

  function toggleMusic() {
    on = !on;
    localStorage.setItem(MUSIC_PREFERENCE, on ? "on" : "off");
    applyMusic();
  }

  /**
   * The auth gate for everything in this group.
   *
   * It lives here rather than per-page because auth is mandatory (#25) — a gate
   * you have to remember to add to each new page is one you will eventually
   * forget. `/auth/finish` is deliberately **outside** this group: it is how a
   * session is obtained, so gating it would deadlock the login.
   *
   * The session is an HttpOnly cookie, so script cannot answer this locally; only the
   * server knows. A 401 is not an error here — `me()` returns null for it, because
   * "nobody is logged in" is an answer rather than a fault. A request that never
   * reaches the server is a different thing, and the shared retry policy waits that
   * one out (see `retryTransient`).
   *
   * Polling while signed out is also how a tab notices a login: opening the
   * bot's link in the same browser sets the cookie, and the next poll simply
   * starts succeeding.
   */
  const session = createQuery(() => ({
    queryKey: ["session"],
    queryFn: me,
    refetchInterval: (q) => (q.state.data ? false : 2000),
  }));

  const authed = $derived(!!session.data);
  const status = $derived<LoginStatus>(loginStatus(session));

  /**
   * Which leg of the meal you are on, or nothing at all.
   *
   * `pick · buy · cook · joy` is the shape of *a meal* — the four things you do to
   * get one on the table, in order. It is not the app's navigation, and on a page
   * that is not part of a meal it says something untrue: standing in your kitchen
   * looking at the pantry, it offers to move you to "cook" as though a meal were
   * underway, and marks one of the four as where you are when you are nowhere in it.
   *
   * So the arc appears while you are walking it, and not before.
   */
  const SECTIONS: Section[] = ["pick", "buy", "cook", "joy"];
  const current = $derived(
    SECTIONS.find((s) => s === page.url.pathname.split("/")[1]),
  );

  /**
   * Which room you are in — the top path segment. It chooses the backdrop, the way it
   * chooses the music (`trackFor`): the photograph behind the kitchens and pick routes
   * lives here, rendered once and held still while the page slides over it, rather than
   * being repeated in each section's layout where it would ride along with the motion.
   */
  const section = $derived(page.url.pathname.split("/")[1]);

  async function signOut() {
    await logout();
    queryClient.clear();
    on = false;
    applyMusic();
  }
</script>

<!-- Two voices so a section change can cross-fade rather than cut. Fetched only on
     demand (preload="none"), so a visitor who never turns the music on downloads
     neither track. -->
<audio bind:this={a} loop preload="none"></audio>
<audio bind:this={b} loop preload="none"></audio>

{#if !authed}
  <Login
    status={status}
    link={botLink()}
    error={session.error instanceof Error ? session.error.message : undefined}
  />
{:else}
  <!-- The room's backdrop, held out here rather than in the page so it stays still
       while the page slides over it (see `pageSlide`). -->
  {#if section === "kitchens"}
    <KitchenBackdrop />
  {:else if section === "pick"}
    <PickBackdrop />
  {/if}

  <!--
    The nav is the heading: `pick · buy · cook · joy` names where you are more
    clearly than an <h1> repeating the same word underneath it would. So the
    line goes first and the page starts below it.
  -->
  {#if current}
    <Nav {current} />
  {/if}

  <div class="mx-auto max-w-2xl px-4 pb-16">
    <div class="flex justify-end gap-3 py-2 text-sm">
      {#if session.data?.username}
        <span class="text-stone-500">@{session.data.username}</span>
      {/if}
      <a href="/kitchens" class="text-stone-500 underline hover:text-stone-900">
        kitchens
      </a>
      {#if session.data?.is_admin}
        <a href="/health" class="text-stone-500 underline hover:text-stone-900">
          health
        </a>
      {/if}
      <button
        onclick={signOut}
        class="text-stone-500 underline hover:text-stone-900"
      >
        Sign out
      </button>
    </div>

    <!-- The page slides; the chrome around it stays. Keying on the path swaps the
         page on navigation, and the leaving and arriving pages share one grid cell so
         neither shoves the other down mid-cross. -->
    <div class="grid">
      {#key page.url.pathname}
        <div
          class="col-start-1 row-start-1"
          in:pageSlide={{ kind: "in" }}
          out:pageSlide={{ kind: "out" }}
        >
          {@render children()}
        </div>
      {/key}
    </div>
  </div>

  <MusicSwitch playing={on} onToggle={toggleMusic} />
{/if}
