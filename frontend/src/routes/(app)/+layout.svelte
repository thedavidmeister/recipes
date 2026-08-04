<script lang="ts">
  import { page } from "$app/state";
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { me, logout, botLink } from "$lib/auth";
  import { consensusRef } from "$lib/buy";
  import { encodeDestination } from "$lib/destination";
  import {
    ADVANCE_RETRY_MS,
    RECONCILE_MS,
    expectedPosition,
    pickFrom,
    planChannel,
    planSection,
    poolFor,
    reconcile,
    type RoomTrack,
  } from "$lib/music";
  import { PickClient } from "$lib/pick";
  import type { LoginStatus, Section } from "$lib/types";
  import { loginStatus } from "$lib/resource";
  import Login from "$lib/components/Login.svelte";
  import Nav from "$lib/components/Nav.svelte";
  import MusicSwitch from "$lib/components/MusicSwitch.svelte";
  import KitchenBackdrop from "$lib/components/KitchenBackdrop.svelte";
  import PickBackdrop from "$lib/components/PickBackdrop.svelte";

  let { children } = $props();
  const queryClient = useQueryClient();

  const MUSIC_PREFERENCE = "recipes:music";
  /** Where a fade settles. A constant, not a setting — see MusicSwitch. */
  const LEVEL = 0.5;
  /** How long a track takes to cross to another, in milliseconds. */
  const FADE = 2500;

  /**
   * **The music** (#88, #121, #125 — and #212/#214, which is most of what follows).
   *
   * Two stories, and the line between them is `channel`:
   *
   * - **A lone device** plays a **pool of tracks per route**, every route played by the
   *   same code — only the list differs (`$lib/music`'s POOLS). Entering a route starts a
   *   random track from its pool and another random one follows when it ends, never the
   *   same twice running. This is exactly what it always was.
   * - **A device in a plan plays the room's soundtrack.** The track and the instant it
   *   started are shared events on the session (#212): the same song, at the position the
   *   shared timeline says, through this device's own measured clock offset. It was a
   *   per-device dice roll before, so two people shopping the same list heard different
   *   songs at different points.
   *
   * The **on/off switch is personal in both**. Sync decides *what* plays and *where in it
   * we are*, never whether this device makes a sound — a device switching on mid-track
   * joins at the room's current position, and the switch is itself the user gesture the
   * browser's autoplay rules want.
   *
   * Moving *between* sections does not cut: one track fades down as the next fades up,
   * like walking from one room into another. A crossfade needs two things playing at
   * once, so there are two voices. Whichever is audible fades out and pauses; the other
   * loads the next track and fades in.
   */
  const NONE: string[] = [];

  /** Is the music switched on. Persisted, and the switch reflects it. */
  let on = $state(false);
  let a: HTMLAudioElement | undefined = $state();
  let b: HTMLAudioElement | undefined = $state();

  // The voice currently carrying `playingSrc`, and the track it is playing. Null when
  // silent — either the section has no pool or the music is off.
  let live: HTMLAudioElement | undefined;
  let playingSrc: string | null = null;

  const wantedPool = $derived(poolFor(page.url.pathname));

  /**
   * **The plan whose room this page belongs to, and which leg of it we are on** (#212).
   *
   * The layout holds the player, so it is the layout that has to know; the *pages* are
   * where the app already keeps the answer, and `planChannel` reads it from the two
   * places they keep it rather than inventing a third — the channel in `/pick/<channel>`'s
   * own URL, and the one that travels with the stashed decision for `buy`/`cook`/`joy`
   * (`$lib/buy`'s `consensusRef`, which is where `getBuyList` and `getCookRecipe` read it
   * too, so the layout and the page under it cannot disagree about which room they are in).
   *
   * Both re-read on navigation, which is what makes a section change ride the room's own
   * move: when the plan decides, every device's screen goes to `buy` (#201's `decided`
   * frame) and every device's music follows it into `buy` at the same moment. The channel
   * does not change across that move, so the socket below is not even reconnected.
   *
   * `null` on `kitchens`, on `/pick` before a plan exists, and for a decision with no
   * plan behind it — and `null` is the whole solo story: no socket, no reconcile loop,
   * and the lone device's random pick untouched.
   */
  const channel = $derived(
    planChannel(page.url.pathname, consensusRef()?.channel),
  );
  const musicSection = $derived(planSection(page.url.pathname));

  /** What the room is playing, per section, exactly as the server last stated it. */
  let roomMusic = $state<Partial<Record<Section, RoomTrack>>>({});
  /** What the server measured this connection's clock to be doing. Read on every
   * comparison rather than folded in once, so an estimate that improves mid-track
   * corrects the position in place. */
  let offsetMs = $state(0);
  let client: PickClient | null = null;
  /** The room's soundtrack for the section this page is on, if it has one. */
  const roomTrack = $derived(
    musicSection ? roomMusic[musicSection] : undefined,
  );

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

  /** Take whatever is audible down and stop it. */
  function fadeOutLive() {
    if (!live) return;
    const old = live;
    fadeTo(old, 0, () => old.pause());
  }

  /** Fade `src` up on the free voice, starting `position` seconds in. */
  function playSrc(src: string, position: number, loop: boolean) {
    const voices = [a, b].filter((v): v is HTMLAudioElement => !!v);
    if (voices.length < 2) return;

    // Prefer a genuinely idle voice. `live` alone is not enough: after a fade to silence
    // `live` is cleared while the old voice is still fading out, and picking that voice
    // would cut its fade mid-decay and leave the truly idle one unused. So skip any voice
    // with a fade in flight first, and only fall back if both are busy.
    const next =
      voices.find((v) => v !== live && !fades.has(v)) ??
      voices.find((v) => v !== live) ??
      voices[0];
    if (next.getAttribute("src") !== src) next.src = src;
    next.currentTime = Math.max(0, position);
    next.loop = loop;
    // A voice that was being nudged back into sync (#214) starts its next track at
    // normal speed: the correction belonged to the track that ended, not to this one.
    next.playbackRate = 1;
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

  /** Fade a fresh random track from `pool` up on the free voice — the lone device's
   * path. One track loops itself seamlessly; a pool of several advances on `ended` (see
   * onEnded), so a real loop point only has to exist when a route has a single track. */
  function playTrack(pool: string[]) {
    playSrc(pickFrom(pool, playingSrc), 0, pool.length === 1);
  }

  /**
   * **Tell the room its soundtrack should move on** (#212) — this section's track has
   * ended, or it has none yet.
   *
   * Several devices raise this at the same moment by design: a track ends on all of them
   * at once, and the server's compare-and-set on `after` picks exactly one while the
   * frame it announces tells the rest what was chosen. What this side owes is not to
   * repeat itself forever — a report can be refused in silence (a watcher's, or one that
   * lost the race and whose answering frame was lost), and without a bound this would
   * raise the same refused report on every check for as long as the track stayed ended.
   */
  let lastReport: { key: string; at: number } | null = null;
  function requestAdvance(section: Section, after: number | null) {
    const key = `${section}:${after ?? "none"}`;
    const now = Date.now();
    if (lastReport?.key === key && now - lastReport.at < ADVANCE_RETRY_MS)
      return;
    lastReport = { key, at: now };
    client?.advanceMusic(section, after);
  }

  /**
   * Make what is playing match the section — the room's, or this device's own — and the
   * on/off preference.
   *
   * Idempotent within a section: if the live track is already the right one it is left
   * alone, so navigating around a section does not restart the music. A different track
   * — or the music going off — crosses to it. Safe to call on every navigation, on the
   * toggle, and from the gesture listeners below: the browser may refuse `play()` until a
   * real gesture, and then the next gesture simply retries.
   */
  function applyMusic() {
    const voices = [a, b].filter((v): v is HTMLAudioElement => !!v);
    if (voices.length < 2) return;

    // The room's soundtrack, when there is a room.
    if (on && channel && musicSection) {
      const room = roomTrack;
      if (!room) {
        // The room has no music in this section yet. Ask for it, and stay silent until
        // it answers: starting a private song here is exactly the bug (#212), and the
        // wait is one round trip.
        requestAdvance(musicSection, null);
        fadeOutLive();
        live = undefined;
        playingSrc = null;
        return;
      }
      if (live && playingSrc === room.track) return;
      fadeOutLive();
      // Joining **mid-track**, at the position the shared timeline says — which is also
      // the whole of rehydrating (#202), since a position was never stored to resume from.
      playSrc(
        room.track,
        expectedPosition(room.started_at, offsetMs, Date.now()),
        false,
      );
      return;
    }

    const pool = on ? wantedPool : NONE;
    if (live && playingSrc !== null && pool.includes(playingSrc)) return;

    fadeOutLive();
    if (pool.length === 0) {
      live = undefined;
      playingSrc = null;
      return;
    }
    playTrack(pool);
  }

  /**
   * A track finished. In a room the next one is the room's — report that this one ended
   * and play whatever comes back, so everybody changes song together instead of drifting
   * apart one track-length at a time. Alone, follow it with another from the same pool;
   * the one that ended is already silent, so that is a faded-in hand-off rather than a
   * crossfade.
   *
   * Only the audible voice advances, and only while the music is on.
   */
  function onEnded(event: Event) {
    if (event.currentTarget !== live || !on) return;
    if (channel && musicSection) {
      requestAdvance(musicSection, roomTrack?.started_at ?? null);
      return;
    }
    const pool = wantedPool;
    if (pool.length > 1) playTrack(pool);
  }

  /**
   * **Heal toward the room** (#214) — one comparison, and the repair it calls for.
   *
   * Playback is physical: a buffering stall, a throttled background tab, a slow decode,
   * a rollover missed while the phone slept. So the expected position is compared against
   * what this element is actually doing, periodically rather than once at track start,
   * and the difference is repaired proportionally — an inaudible `playbackRate` nudge for
   * drift, a hard seek for a stall or a wake, a load for a track this device missed the
   * change to.
   *
   * **Strictly local.** Every branch below moves *this* device. The one thing it can say
   * to the room is that the room's own track has ended, which is a fact about the shared
   * timeline rather than about this device's trouble — and it is answered by one
   * compare-and-set that several devices are racing anyway.
   */
  function healTowardTheRoom() {
    const room = roomTrack;
    const section = musicSection;
    if (!on || !channel || !room || !section || !live) return;
    const repair = reconcile(
      room,
      {
        track: playingSrc,
        currentTime: live.currentTime,
        duration: live.duration,
      },
      expectedPosition(room.started_at, offsetMs, Date.now()),
    );
    switch (repair.kind) {
      case "load":
        fadeOutLive();
        playSrc(repair.track, repair.position, false);
        break;
      case "seek":
        live.currentTime = repair.position;
        live.playbackRate = 1;
        break;
      case "nudge":
        live.playbackRate = repair.rate;
        break;
      case "hold":
        if (live.playbackRate !== 1) live.playbackRate = 1;
        break;
      case "advance":
        requestAdvance(section, room.started_at);
        break;
    }
  }

  // React to the section (pool) changing, to the room's soundtrack moving, and to the
  // on/off toggle.
  $effect(() => {
    void wantedPool;
    void channel;
    void roomTrack;
    void on;
    applyMusic();
  });

  /**
   * The plan's room, so the soundtrack's events reach this player live (#212).
   *
   * The layout owns the player, so it owns the player's connection. It listens for the
   * two frames the music is made of and ignores everything else the room says — which is
   * what `PickHandlers`' optional handlers are for: one socket serves the pick, `buy`,
   * `cook` and now the player, and none of them writes empty functions for another's
   * traffic.
   *
   * Opened **only when there is a plan**. No channel, no socket: a lone device's music
   * is its own and has nothing to say to anybody.
   */
  $effect(() => {
    const ch = channel;
    if (!ch) return;
    const c = new PickClient(ch, {
      onMusic: (section, track, started_at) => {
        roomMusic[section] = { section, track, started_at };
      },
      onTimeSync: (offset) => {
        offsetMs = offset;
      },
      // A report needs an open socket to leave on — `PickClient` drops one silently
      // otherwise — so the first one is raised when there is one, and a reconnect re-arms
      // rather than sitting out the throttle below on a frame that never left.
      onStatus: (status) => {
        if (status !== "open") return;
        lastReport = null;
        applyMusic();
      },
    });
    c.start();
    client = c;
    return () => {
      c.stop();
      client = null;
      // The room's state describes a socket's plan; a different plan re-reads it, and a
      // stale track would otherwise be played into the next one.
      roomMusic = {};
      offsetMs = 0;
      lastReport = null;
    };
  });

  // The reconcile loop (#214) — **and there is none without a plan**, because solo music
  // has no shared truth to converge on. It also stops when the music is off: a silent
  // device has nothing to heal, and it rejoins at the room's current position the moment
  // the switch goes back on.
  $effect(() => {
    if (!channel || !on) return;
    const timer = setInterval(healTowardTheRoom, RECONCILE_MS);
    return () => clearInterval(timer);
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
   * The page the login was reached from, so signing in comes back to it (#206).
   *
   * The case that made this necessary is a scanned invite: a QR opens the system
   * browser, which holds no session however signed in the phone's Telegram is, so
   * the invite shows this screen — and a login that landed at home dropped the plan
   * on the floor. It goes out as the bot's deep-link payload and comes back beside
   * the secret in the bot's reply.
   *
   * Home carries nothing, because that is where a bare `/start` already lands, and
   * `carriesDestination` is what the screen says so out loud — it is false for a
   * path too long for Telegram to hold, and promising a return that will not happen
   * is worse than not offering one.
   */
  const returnTo = $derived(page.url.pathname + page.url.search);
  const carriesDestination = $derived(encodeDestination(returnTo) !== null);

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
   * lives here, rendered once for the whole room, rather than being repeated in each
   * section's layout.
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
     nothing. `loop` is set per track (playSrc): a lone device's single-track route loops
     itself, everything else advances on `ended` — and in a room that always means asking
     the room for the next track rather than picking one. -->
<audio bind:this={a} preload="none" onended={onEnded}></audio>
<audio bind:this={b} preload="none" onended={onEnded}></audio>

{#if !authed}
  <Login
    {status}
    link={botLink(returnTo)}
    returning={carriesDestination}
    error={session.error instanceof Error ? session.error.message : undefined}
  />
{:else}
  <!-- The room's backdrop, rendered once here for the whole room (see `section`)
       rather than repeated in each section's layout. -->
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
    <!--
      Account links: chrome, sitting above the page content. The `kitchens` link
      belongs here and nowhere else. It is a peer of `health` and `Sign out` — the
      same row on every page of the app, including ones no kitchen is above — so it
      reads as a utility rather than a parent. A kitchen page states the same
      destination in its own words, quietly, at the bottom: "Switch kitchen".
    -->
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

  <MusicSwitch playing={on} onToggle={toggleMusic} />
{/if}
