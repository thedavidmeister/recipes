<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { resource } from "$lib/resource";
  import { getCookRecipe } from "$lib/cook";
  import { me } from "$lib/auth";
  import { getLobby, PickClient } from "$lib/pick";
  import { isWatching } from "$lib/roster";
  import type { RunningTimer } from "$lib/session-events";
  import {
    loadDeadlines,
    localTimers,
    saveDeadlines,
    sharedTimers,
    type Deadlines,
    type StepTimer,
  } from "$lib/steps";
  import Cook from "$lib/components/Cook.svelte";

  /**
   * `cook` (#36) — the picked recipe in full, to follow while cooking.
   *
   * Reads the pick's decision (the consensus recipe) and shows the method as the
   * model's step DAG (#74/#75/#76). The page owns the query AND the live timer
   * machinery — ticking each second, sounding + notifying at zero — and passes the
   * derived per-step `timers` down, so `Cook` renders a pure view.
   *
   * **A timer belongs to the plan, not to the phone (#208).** Everyone in a cook is
   * standing over the same pot, so the countdown on it is the room's: whoever taps
   * Start records the instant, the server normalises it through that person's measured
   * clock offset and holds it, and every screen renders the same deadline. Before this,
   * the tapper got a countdown and everybody else's screen showed the step idle.
   *
   * The split is `buy`'s, and the same line: **`shared = !!channel`**. With a plan
   * behind the decision the timers come over the session socket and a start is an event
   * raised through the framework (`$lib/session-events`); with no plan the existing
   * device-local path is untouched, down to its `localStorage` key — a cook you started
   * alone has no room to sync with, and refusing to time it over that would be absurd.
   *
   * **Alerts stay local.** The deadline is shared; the beep is not. Each device sounds
   * its own alarm when the shared deadline crosses, and a device that arrives to a timer
   * already finished stays quiet, because the moment it would be announcing has passed.
   */
  const recipe = resource(() => ({
    queryKey: ["cook"],
    queryFn: () => getCookRecipe(),
    staleTime: Infinity,
  }));

  const channel = $derived(recipe.data?.channel);
  const shared = $derived(!!channel);

  // ---- the shared half: the plan's timers, and this connection's clock ----------

  /** The room's timers for this recipe, exactly as the server last stated them —
   * replaced whole on every frame, never merged (the `buy` rule: a client that missed
   * one is corrected by the next instead of drifting further). */
  let roomTimers = $state<RunningTimer[]>([]);
  /** What the server measured this connection's clock to be doing. Read on every tick
   * rather than folded into the deadlines once, so an estimate that improves mid-simmer
   * corrects the countdown in place. */
  let offsetMs = $state(0);
  let client: PickClient | null = null;

  // Who is looking, and who is cooking — so a watcher is offered nothing (#200). The
  // layout has already fetched the session, so it is a cache read rather than a request.
  const session = createQuery(() => ({ queryKey: ["session"], queryFn: me }));
  const lobby = resource(() => ({
    queryKey: ["lobby", channel],
    queryFn: () => (channel ? getLobby(channel) : Promise.resolve(null)),
    enabled: !!channel,
    staleTime: Infinity,
  }));

  /**
   * Watching, not cooking (#180/#200) — and reachable here, because the `decided` frame
   * goes to the **room**: a watcher is carried out of the pick with the decision stashed
   * like everybody else, through `buy` and on to this screen.
   *
   * The same three-way guard the pick uses, out of the same pure module, so "watching"
   * means one thing across the app. A watcher sees every countdown and starts nothing —
   * and the server says so too (`events::Guard::SeatedInStartedPlan`), so this is the
   * refusal said *before* the fact rather than the only place it is said.
   */
  const watching = $derived(
    isWatching({
      started: lobby.data?.started,
      roster: lobby.data?.voters,
      viewer: session.data?.telegram_user_id,
    }),
  );

  // ---- the solo half: this device's own deadlines ------------------------------

  // step id → deadline (unix ms), on this device's clock and nobody else's.
  let deadlines = $state<Deadlines>({});

  // ---- both --------------------------------------------------------------------

  /** Which steps this device has already alerted for. Local by design: sync the
   * deadline, not the beep. */
  let fired = $state<Record<number, boolean>>({});
  /** Which steps this device has *seen running*. A timer only rings for a screen that
   * watched it cross — one that was already finished when this screen learned about it
   * (a rehydrated plan, a reload mid-cook) shows as done and stays quiet. */
  let armed = $state<Record<number, boolean>>({});
  /** Ticks each second so the countdowns re-derive. */
  let now = $state(Date.now());

  // Restore this recipe's device-local timers when it arrives. Only on the solo path:
  // with a plan behind the cook the timers are the plan's, and reading a stale copy out
  // of this browser would put a second, private answer on screen beside the room's.
  $effect(() => {
    const r = recipe.data;
    if (!r || r.channel) return;
    deadlines = loadDeadlines(r.source, r.id);
  });

  // The meal's room, so a peer's start or dismiss lands here live — the same socket the
  // pick and `buy` use, listening only for the frames this page is about.
  $effect(() => {
    const r = recipe.data;
    if (!r?.channel) return;
    const c = new PickClient(r.channel, {
      onTimers: (source, id, incoming) => {
        // Another recipe's timers in the same plan are not this screen's business.
        if (source !== r.source || id !== r.id) return;
        roomTimers = incoming;
        // Re-anchor the tick: without it the first render runs against a `now` up to a
        // second stale and flashes seconds+1 until the next tick.
        now = Date.now();
      },
      onTimeSync: (offset) => {
        offsetMs = offset;
      },
    });
    c.start();
    client = c;
    return () => {
      c.stop();
      client = null;
    };
  });

  // Tick every second while mounted.
  $effect(() => {
    const iv = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(iv);
  });

  /**
   * What the screen shows, from whichever half owns the truth.
   *
   * The **only** place the two paths meet, and neither knows about the other: a shared
   * timer is the room's instant translated through this connection's offset, a local one
   * is this browser's own deadline. Both come out of `$lib/steps` in the same shape, on
   * the same "done is a reading of the deadline" rule.
   */
  const timers = $derived<Record<number, StepTimer>>(
    shared
      ? sharedTimers(roomTimers, offsetMs, now)
      : localTimers(deadlines, now),
  );

  // Alert when a timer this device watched running crosses its deadline — once, then it
  // stays done until somebody dismisses it.
  $effect(() => {
    for (const [idStr, t] of Object.entries(timers)) {
      const id = Number(idStr);
      if (!t.done) {
        if (!armed[id]) armed = { ...armed, [id]: true };
      } else if (armed[id] && !fired[id]) {
        fired = { ...fired, [id]: true };
        alertDone(id);
      }
    }
  });

  function persist() {
    const r = recipe.data;
    if (r) saveDeadlines(r.source, r.id, deadlines);
  }

  function startTimer(id: number) {
    const r = recipe.data;
    const step = r?.steps.find((s) => s.id === id);
    if (!r || !step || step.seconds == null) return;
    // A watcher has no buttons to press, and this is the same sentence said where the
    // frame would be sent: the server refuses the write and the socket has nowhere to
    // answer, so an event raised from here would be swallowed in silence rather than
    // refused out loud.
    if (watching) return;
    // Ask for Notification permission on the first start (gated behind the tap, never
    // on load) so a finished timer still notifies while the tab is backgrounded.
    requestNotify();

    if (shared) {
      // The room's timer. **No duration is sent** — the recipe's own is read on the
      // server (`timers::step_seconds`); this names the step, and the instant of the tap
      // rides on the envelope. Nothing is set optimistically either: the deadline does
      // not exist until it has been recorded, and a countdown running against a number
      // nobody else has is precisely the bug this replaced.
      client?.event({
        kind: "timer_start",
        source: r.source,
        id: r.id,
        step: id,
      });
      return;
    }

    // Anchor `now` to the start instant, and derive the deadline from the same one:
    // the 1s tick otherwise leaves `now` lagging Date.now(), flashing seconds+1 on
    // the first render until the next tick.
    const start = Date.now();
    now = start;
    deadlines = { ...deadlines, [id]: start + step.seconds * 1000 };
    fired = { ...fired, [id]: false };
    armed = { ...armed, [id]: false };
    persist();
  }

  function dismissTimer(id: number) {
    const r = recipe.data;
    if (!r || watching) return;
    const { [id]: _f, ...restFired } = fired;
    const { [id]: _a, ...restArmed } = armed;
    fired = restFired;
    armed = restArmed;

    if (shared) {
      client?.event({
        kind: "timer_dismiss",
        source: r.source,
        id: r.id,
        step: id,
      });
      return;
    }

    const { [id]: _d, ...restDeadlines } = deadlines;
    deadlines = restDeadlines;
    persist();
  }

  function requestNotify() {
    try {
      if (
        typeof Notification !== "undefined" &&
        Notification.permission === "default"
      ) {
        void Notification.requestPermission();
      }
    } catch {
      // Notifications unavailable — the on-screen flag + sound still fire.
    }
  }

  function alertDone(id: number) {
    beep();
    const step = recipe.data?.steps.find((s) => s.id === id);
    try {
      if (
        typeof Notification !== "undefined" &&
        Notification.permission === "granted"
      ) {
        new Notification("Timer done", {
          body: step?.text ?? "A step's timer finished.",
        });
      }
    } catch {
      // Ignore — the visual flag is the fallback.
    }
  }

  function beep() {
    try {
      const Ctx =
        window.AudioContext ??
        (window as unknown as { webkitAudioContext?: typeof AudioContext })
          .webkitAudioContext;
      if (!Ctx) return;
      const ctx = new Ctx();
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.type = "sine";
      osc.frequency.value = 880;
      gain.gain.setValueAtTime(0.15, ctx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.6);
      osc.start();
      osc.stop(ctx.currentTime + 0.6);
      // Release the context when the tone ends: a browser caps concurrent hardware
      // AudioContexts (Chrome at 6), so leaking one per alert would eventually throw
      // and silence the beep.
      osc.onended = () => void ctx.close();
    } catch {
      // No audio context — silent.
    }
  }
</script>

<Cook
  status={recipe.status}
  {timers}
  {watching}
  recipe={recipe.data}
  error={recipe.error}
  onStartTimer={startTimer}
  onDismissTimer={dismissTimer}
/>
