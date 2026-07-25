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
   * The music (#88, #121, #125): a **pool of tracks per section**, every section played
   * by the same code — only the list differs. Entering a section starts a random track
   * from its pool, and when one ends another random one from the same pool follows —
   * never the same twice running — so a section cycles through its set instead of
   * looping a single song. A pool of one track just loops it; an empty pool is silence,
   * where a section sits until its tracks arrive. Moving *between* sections does not cut:
   * one track fades down as the next fades up, like walking from one room into another.
   *
   * A crossfade needs two things playing at once, so there are two voices. Whichever is
   * audible fades out and pauses; the other loads the next track and fades in.
   */
  const NONE: string[] = [];
  const POOLS: Record<string, string[]> = {
    kitchens: [
      "/music/title-1.mp3",
      "/music/title-2.mp3",
      "/music/title-3.mp3",
      "/music/title-4.mp3",
    ],
    pick: ["/pick.mp3"],
    buy: [
      "/music/buy-1.mp3",
      "/music/buy-2.mp3",
      "/music/buy-3.mp3",
      "/music/buy-4.mp3",
    ],
    // Their own tracks are still to come; empty until then.
    cook: [],
    joy: [],
  };
  function poolFor(pathname: string): string[] {
    return POOLS[pathname.split("/")[1]] ?? NONE;
  }

  /**
   * A random track from `pool`, avoiding `exclude` so a pool of several does not repeat
   * back to back. The rule relaxes when there is nothing else to honour it with — a
   * single-track pool, or one whose remaining tracks are all `exclude` — and returns
   * what there is, so a lone track just keeps playing.
   */
  function pickFrom(pool: string[], exclude: string | null): string {
    const others = pool.filter((track) => track !== exclude);
    const options = others.length ? others : pool;
    return options[Math.floor(Math.random() * options.length)];
  }

  const wantedPool = $derived(poolFor(page.url.pathname));

  /** Is the music switched on. Persisted, and the switch reflects it. */
  let on = $state(false);
  let a: HTMLAudioElement | undefined = $state();
  let b: HTMLAudioElement | undefined = $state();

  // The voice currently carrying `playingSrc`, and the track it is playing. Null when
  // silent — either the section has no pool or the music is off.
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

  /** Fade a fresh random track from `pool` up on the free voice. */
  function playTrack(pool: string[]) {
    const voices = [a, b].filter((v): v is HTMLAudioElement => !!v);
    if (voices.length < 2) return;

    const src = pickFrom(pool, playingSrc);
    // Prefer a genuinely idle voice. `live` alone is not enough: after a fade to silence
    // `live` is cleared while the old voice is still fading out, and picking that voice
    // would cut its fade mid-decay and leave the truly idle one unused. So skip any voice
    // with a fade in flight first, and only fall back if both are busy.
    const next =
      voices.find((v) => v !== live && !fades.has(v)) ??
      voices.find((v) => v !== live) ??
      voices[0];
    if (next.getAttribute("src") !== src) next.src = src;
    next.currentTime = 0;
    // One track loops itself seamlessly; a pool of several advances on `ended` (see
    // onEnded), so a real loop point only has to exist when a section has a single track.
    next.loop = pool.length === 1;
    next.volume = 0;
    next.play().then(
      () => {
        fadeTo(next, LEVEL);
        live = next;
        playingSrc = src;
      },
      () => {
        // Refused: no gesture credited yet. A gesture listener will retry.
      },
    );
  }

  /**
   * Make what is playing match the section — and the on/off preference.
   *
   * Idempotent within a section: if the live track already belongs to the wanted pool
   * it is left alone, so navigating around a section does not restart the music. A
   * different pool — or the music going off — crosses to it. Safe to call on every
   * navigation, on the toggle, and from the gesture listeners below: the browser may
   * refuse `play()` until a real gesture, and then the next gesture simply retries.
   */
  function applyMusic() {
    const voices = [a, b].filter((v): v is HTMLAudioElement => !!v);
    if (voices.length < 2) return;

    const pool = on ? wantedPool : NONE;
    if (live && playingSrc !== null && pool.includes(playingSrc)) return;

    if (live) {
      const old = live;
      fadeTo(old, 0, () => old.pause());
    }
    if (pool.length === 0) {
      live = undefined;
      playingSrc = null;
      return;
    }
    playTrack(pool);
  }

  /**
   * A track finished: follow it with another from the same pool. The one that ended is
   * already silent, so this is a faded-in hand-off rather than a crossfade. Only the
   * audible voice advances, and only while the music is on and still in a pooled section
   * with more than one track (a lone track loops itself and never ends).
   */
  function onEnded(event: Event) {
    if (event.currentTarget !== live || !on) return;
    const pool = wantedPool;
    if (pool.length > 1) playTrack(pool);
  }

  // React to the section (pool) changing and to the on/off toggle.
  $effect(() => {
    void wantedPool;
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
   * chooses the music (`poolFor`): the photograph behind the kitchens and pick routes
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
     nothing. `loop` is set per track (playTrack): a lone track loops itself, a pool
     advances on `ended`. -->
<audio bind:this={a} preload="none" onended={onEnded}></audio>
<audio bind:this={b} preload="none" onended={onEnded}></audio>

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

  <!-- Account links: chrome, kept in a persistent centred column above the page
       rather than riding along with the slide. -->
  <div class="mx-auto max-w-2xl px-4">
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
  </div>

  <!-- The page slides; the chrome around it stays. The sliding element spans the full
       viewport (the content column sits centred inside it), so `translateX(100%)`
       carries the page right off the screen rather than only across its own narrow
       column — and `overflow-x-clip` clips that travel at the viewport edge, so it
       grows no scrollbar. The leaving and arriving pages share one grid cell so neither
       shoves the other down mid-cross. -->
  <div class="grid overflow-x-clip">
    {#key page.url.pathname}
      <div
        class="col-start-1 row-start-1"
        in:pageSlide={{ kind: "in" }}
        out:pageSlide={{ kind: "out" }}
      >
        <div class="mx-auto max-w-2xl px-4 pb-16">
          {@render children()}
        </div>
      </div>
    {/key}
  </div>

  <MusicSwitch playing={on} onToggle={toggleMusic} />
{/if}
