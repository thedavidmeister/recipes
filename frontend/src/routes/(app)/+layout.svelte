<script lang="ts">
  import { page } from "$app/state";
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { me, logout, botLink } from "$lib/auth";
  import type { LoginStatus, Section } from "$lib/types";
  import Login from "$lib/components/Login.svelte";
  import Nav from "$lib/components/Nav.svelte";
  import MusicSwitch from "$lib/components/MusicSwitch.svelte";

  let { children } = $props();
  const queryClient = useQueryClient();

  const MUSIC_PREFERENCE = "recipes:music";
  const MUSIC_VOLUME = "recipes:music-volume";
  const DEFAULT_VOLUME = 0.5;
  /** How long the track takes to come up to volume, in milliseconds. */
  const FADE = 2500;

  let audio: HTMLAudioElement | undefined = $state();
  let playing = $state(false);

  /**
   * The remembered level. Read carefully: a missing key must fall through to the
   * default, and `Number(null)` is `0` — a perfectly valid volume — so an absent
   * setting would otherwise read as silence and look like the music was broken.
   */
  function storedVolume(): number {
    try {
      const raw = localStorage.getItem(MUSIC_VOLUME);
      if (raw === null) return DEFAULT_VOLUME;
      const level = Number(raw);
      return Number.isFinite(level) && level >= 0 && level <= 1
        ? level
        : DEFAULT_VOLUME;
    } catch {
      return DEFAULT_VOLUME;
    }
  }

  let volume = $state(storedVolume());
  let fading: number | undefined;

  function stopFading() {
    if (fading !== undefined) cancelAnimationFrame(fading);
    fading = undefined;
  }

  /**
   * Come up from silence rather than landing at full volume.
   *
   * A loop that starts at its own level is a jolt — the first thing the app does is
   * shout — and the point of it is atmosphere. Reading `volume` on every frame rather
   * than capturing it up front means a drag mid-fade ramps toward the new level
   * instead of finishing to the old one.
   */
  function fadeIn(el: HTMLAudioElement) {
    stopFading();
    const startedAt = performance.now();
    el.volume = 0;
    const step = () => {
      const through = Math.min(1, (performance.now() - startedAt) / FADE);
      el.volume = volume * through;
      fading = through < 1 ? requestAnimationFrame(step) : undefined;
    };
    fading = requestAnimationFrame(step);
  }

  /**
   * A drag is an instruction about *now*, so it ends any fade in progress rather than
   * fighting it for control of the same property.
   */
  function setVolume(level: number) {
    volume = level;
    stopFading();
    if (audio) audio.volume = level;
    try {
      localStorage.setItem(MUSIC_VOLUME, String(level));
    } catch {
      // No storage (private mode): the level holds for this visit.
    }
  }

  /**
   * The music (#88), owned here so it survives every navigation inside the app — the
   * track keeps going as you move from pick to buy to a kitchen, and only a reload or
   * a sign-out stops it.
   *
   * Nothing plays unasked. A browser grants audio only to a real user gesture, so the
   * switch is the only thing that reliably starts it — and it is called straight from
   * the click rather than from an effect scheduled after it, which may already have
   * fallen outside the window where the gesture counts.
   *
   * What the policy actually requires is a gesture *somewhere* — any click or key,
   * not a particular button — or an origin the browser has learned you play audio on
   * (Chrome's media engagement score, which is why YouTube appears to autoplay and a
   * site you have never used does not). So the music asks immediately, in case this
   * browser already trusts us, and otherwise starts on the first thing you do here.
   * Whichever lands, it is the same track a moment later.
   *
   * Switching it off is remembered and checked again at the moment of the gesture, so
   * a click after you turned it off does not turn it back on.
   */
  $effect(() => {
    const el = audio;
    if (!el) return;

    const wanted = () => localStorage.getItem(MUSIC_PREFERENCE) !== "off";
    const attempt = () => {
      if (!wanted()) return;
      el.play().then(
        () => {
          playing = true;
          fadeIn(el);
        },
        () => {
          // Refused: no gesture credited yet. The listeners below are the next chance.
        },
      );
    };

    attempt();
    window.addEventListener("pointerdown", attempt, { once: true });
    window.addEventListener("keydown", attempt, { once: true });
    return () => {
      window.removeEventListener("pointerdown", attempt);
      window.removeEventListener("keydown", attempt);
    };
  });

  function toggleMusic() {
    if (!audio) return;
    if (playing) {
      stopFading();
      audio.pause();
      playing = false;
      localStorage.setItem(MUSIC_PREFERENCE, "off");
      return;
    }
    localStorage.setItem(MUSIC_PREFERENCE, "on");
    const el = audio;
    el.play().then(
      () => {
        playing = true;
        fadeIn(el);
      },
      () => (playing = false),
    );
  }

  /**
   * The auth gate for everything in this group.
   *
   * It lives here rather than per-page because auth is mandatory (#25) — a gate
   * you have to remember to add to each new page is one you will eventually
   * forget. `/auth/finish` is deliberately **outside** this group: it is how a
   * session is obtained, so gating it would deadlock the login.
   *
   * The session is an HttpOnly cookie, so script cannot answer this locally;
   * only the server knows. `retry: false` because a 401 is a legitimate answer
   * ("nobody is logged in"), not a failure worth retrying.
   *
   * Polling while signed out is also how a tab notices a login: opening the
   * bot's link in the same browser sets the cookie, and the next poll simply
   * starts succeeding.
   */
  const session = createQuery(() => ({
    queryKey: ["session"],
    queryFn: me,
    retry: false,
    refetchInterval: (q) => (q.state.data ? false : 2000),
  }));

  const authed = $derived(!!session.data);
  const loginStatus = $derived<LoginStatus>(
    session.isError ? "error" : session.isPending ? "checking" : "idle",
  );

  // The first path segment is the section. Anything else has no business here.
  const current = $derived(
    (page.url.pathname.split("/")[1] || "pick") as Section,
  );

  async function signOut() {
    await logout();
    queryClient.clear();
    stopFading();
    audio?.pause();
    playing = false;
  }
</script>

<!-- Mounted from the start but fetched only on demand, so the track costs a visitor
     who never presses Start exactly nothing. -->
<audio bind:this={audio} src="/kitchen.mp3" loop preload="none"></audio>

{#if !authed}
  <Login
    status={loginStatus}
    link={botLink()}
    error={session.error instanceof Error ? session.error.message : undefined}
  />
{:else}
  <!--
    The nav is the heading: `pick · buy · cook · joy` names where you are more
    clearly than an <h1> repeating the same word underneath it would. So the
    line goes first and the page starts below it.
  -->
  <Nav {current} />

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

    {@render children()}
  </div>

  <MusicSwitch {playing} {volume} onToggle={toggleMusic} onVolume={setVolume} />
{/if}
