<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { getCookRecipe } from "$lib/cook";
  import { consensusRef } from "$lib/buy";
  import {
    loadDeadlines,
    saveDeadlines,
    type Deadlines,
    type StepTimer,
  } from "$lib/steps";
  import type { CookStatus } from "$lib/types";
  import Cook from "$lib/components/Cook.svelte";

  /**
   * `cook` (#36) — the picked recipe in full, to follow while cooking.
   *
   * Reads the pick's decision (the consensus recipe) and shows the method as the
   * model's step DAG (#74/#75/#76). The page owns the query AND the live timer
   * machinery — ticking each second, sounding + notifying at zero, persisting a
   * running timer per recipe so a reload mid-cook keeps counting — and passes the
   * derived per-step `timers` down, so `Cook` renders a pure view.
   */
  const recipe = createQuery(() => ({
    queryKey: ["cook"],
    queryFn: () => getCookRecipe(),
    staleTime: Infinity,
  }));

  const status = $derived<CookStatus>(
    recipe.isError ? "error" : recipe.isPending ? "pending" : "ready",
  );

  // Running timers: step id → deadline (unix ms), plus which have already fired.
  // `now` ticks each second so the countdowns re-derive.
  let deadlines = $state<Deadlines>({});
  let fired = $state<Record<number, boolean>>({});
  let now = $state(Date.now());

  // Restore this recipe's timers when it arrives; one already past its deadline shows
  // as done rather than re-alerting on load.
  $effect(() => {
    if (!recipe.data) return;
    const ref = consensusRef();
    if (!ref) return;
    const restored = loadDeadlines(ref.source, ref.id);
    const done: Record<number, boolean> = {};
    for (const [id, deadline] of Object.entries(restored)) {
      if (Date.now() >= deadline) done[Number(id)] = true;
    }
    deadlines = restored;
    fired = done;
  });

  // Tick every second while mounted.
  $effect(() => {
    const iv = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(iv);
  });

  // Alert when a running timer crosses its deadline — once, then it stays done.
  $effect(() => {
    for (const [idStr, deadline] of Object.entries(deadlines)) {
      const id = Number(idStr);
      if (now >= deadline && !fired[id]) {
        fired = { ...fired, [id]: true };
        alertDone(id);
      }
    }
  });

  const timers = $derived.by(() => {
    const out: Record<number, StepTimer> = {};
    for (const [idStr, deadline] of Object.entries(deadlines)) {
      const id = Number(idStr);
      out[id] = {
        remaining: Math.max(0, Math.ceil((deadline - now) / 1000)),
        done: !!fired[id],
      };
    }
    return out;
  });

  function persist() {
    const ref = consensusRef();
    if (ref) saveDeadlines(ref.source, ref.id, deadlines);
  }

  function startTimer(id: number) {
    const step = recipe.data?.steps.find((s) => s.id === id);
    if (!step || step.seconds == null) return;
    // Ask for Notification permission on the first start (gated behind the tap, never
    // on load) so a finished timer still notifies while the tab is backgrounded.
    requestNotify();
    // Anchor `now` to the start instant, and derive the deadline from the same one:
    // the 1s tick otherwise leaves `now` lagging Date.now(), flashing seconds+1 on
    // the first render until the next tick.
    const start = Date.now();
    now = start;
    deadlines = { ...deadlines, [id]: start + step.seconds * 1000 };
    fired = { ...fired, [id]: false };
    persist();
  }

  function dismissTimer(id: number) {
    const { [id]: _d, ...restDeadlines } = deadlines;
    const { [id]: _f, ...restFired } = fired;
    deadlines = restDeadlines;
    fired = restFired;
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
  {status}
  {timers}
  recipe={recipe.data}
  error={recipe.error instanceof Error ? recipe.error.message : undefined}
  onStartTimer={startTimer}
  onDismissTimer={dismissTimer}
/>
