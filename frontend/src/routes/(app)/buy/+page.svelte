<script lang="ts">
  import { resource } from "$lib/resource";
  import {
    getBuyList,
    getChecks,
    loadChecks,
    saveChecks,
    setCheck,
    type BuyCheck,
  } from "$lib/buy";
  import { NOBODY, type Tick } from "$lib/shopping";
  import { PickClient } from "$lib/pick";
  import Buy from "$lib/components/Buy.svelte";

  /**
   * `buy` (#36) — the shopping checklist for the recipe the group picked.
   *
   * The step after `pick`: a pick decides on one recipe (consensus) and stashes it,
   * so this reads that decision and lists its ingredients to tick off. The page
   * owns the queries and the checklist state; `Buy` renders. The recipe itself is
   * read client-direct from Turso (the corpus is public), so a lapsed session
   * doesn't 401 it — the layout already gates the shell.
   *
   * The ticks belong to the meal, not to this browser (#131). They live in the meal
   * session beside the pick that chose the recipe, so two people shopping the same
   * dinner see one list — and each ticked line says who got it. A tick is a write to
   * the session and an announcement to its room, so a peer's tick lands here without
   * a refresh.
   *
   * The exception is a decision with no session behind it: nothing to write to and
   * nobody to attribute to, so the list falls back to this device's own
   * (`loadChecks`/`saveChecks`) and says so on screen. Ticking is never refused over
   * the lack of a group — a shopping list you cannot tick is not a shopping list.
   *
   * Nothing here asks for the **pantry pre-ticks** (#156) and nothing here computes
   * them. A list arrives already carrying whatever the plan's kitchen had, because
   * the server seeds it the first time anyone asks for it — the browser could not do
   * this anyway (it holds no roster, no kitchen and no write token), and a second
   * implementation of the matching rule in TypeScript would be a second chance to
   * disagree with `recipe_core::pantry`. The same absence is why the device-local
   * path has none: no session, no plan, no kitchen, no pantry.
   */
  const list = resource(() => ({
    queryKey: ["buy"],
    queryFn: () => getBuyList(),
    staleTime: Infinity,
  }));

  const shared = $derived(!!list.data?.channel);

  // The shared list as the server last stated it — replaced whole on every answer
  // and every room announcement, never merged: a tick can take an item off somebody
  // else (last writer wins), so a delta would be a lie.
  let checks = $state<BuyCheck[]>([]);
  // Taps that have not been confirmed yet: index → the state this client asked for.
  // Kept beside the server's answer rather than folded into it, so a failure has
  // something true to fall back to and never has to guess who ticked what.
  let inFlight = $state<Record<number, boolean>>({});
  // The device-local list, for a decision with no session to attribute to.
  let localChecked = $state<Record<number, true>>({});
  let tickError = $state<string | undefined>();

  /**
   * What the screen shows: where each ticked line's tick came from.
   *
   * `NOBODY` is a tick with nothing behind it — either a tap still in flight (the
   * server has not said whose it is yet) or the device-local list, which has no
   * whose at all. Both read the same way on purpose: got, unattributed. A pantry
   * pre-tick (#156) is also nobody's, but it is *not* the same thing and does not
   * collapse into this: it carries the entry that answered for the line, so the row
   * can say why it is ticked. The server sends it that way and it is passed through
   * unchanged.
   */
  const ticks = $derived.by<Record<number, Tick>>(() => {
    if (!shared) {
      return Object.fromEntries(
        Object.keys(localChecked).map((i) => [i, NOBODY]),
      );
    }
    const out: Record<number, Tick> = {};
    for (const c of checks) out[c.index] = { by: c.by, pantry: c.pantry };
    for (const [i, want] of Object.entries(inFlight)) {
      // A tick in flight shows as ticked-but-unattributed unless the server has
      // already named an owner for that line — this client does not hold an
      // identity to put there, and inventing one is how a row claims the wrong
      // person for a moment.
      if (want) out[Number(i)] ??= NOBODY;
      else delete out[Number(i)];
    }
    return out;
  });

  // Load this recipe's ticks when it arrives (or changes). The two paths are read
  // the same way and differ only in where from.
  $effect(() => {
    const r = list.data;
    checks = [];
    inFlight = {};
    localChecked = {};
    tickError = undefined;
    if (!r) return;
    if (r.channel) {
      const ch = r.channel;
      void getChecks(ch, r.source, r.id)
        .then((l) => {
          checks = l.checks;
        })
        .catch((e: unknown) => {
          tickError =
            e instanceof Error ? e.message : "Couldn't open the shopping list.";
        });
    } else {
      const map: Record<number, true> = {};
      for (const i of loadChecks(r.source, r.id)) map[i] = true;
      localChecked = map;
    }
  });

  // The meal's room, so a peer's tick lands here live — the same socket the pick
  // uses, listening only for the frame this page is about.
  $effect(() => {
    const r = list.data;
    if (!r?.channel) return;
    const client = new PickClient(r.channel, {
      onBuy: (source, id, incoming) => {
        // Another recipe's list in the same meal is not this screen's business.
        if (source !== r.source || id !== r.id) return;
        checks = incoming;
      },
    });
    client.start();
    return () => client.stop();
  });

  /**
   * Tick or untick a line.
   *
   * Optimistic, then confirmed: a tap has to feel like a tap in a supermarket
   * aisle, so the row moves at once and the round trip catches up. If the write
   * fails the row goes back to what the server last said and the reason is shown —
   * a line that looks got but is not is how somebody comes home without the flour.
   */
  async function toggle(index: number) {
    const r = list.data;
    if (!r) return;
    tickError = undefined;

    if (!r.channel) {
      const next = { ...localChecked };
      if (next[index]) delete next[index];
      else next[index] = true;
      localChecked = next;
      saveChecks(r.source, r.id, Object.keys(next).map(Number));
      return;
    }

    const want = !(index in ticks);
    inFlight = { ...inFlight, [index]: want };
    try {
      const l = await setCheck(r.channel, r.source, r.id, index, want);
      checks = l.checks;
    } catch (e) {
      tickError =
        e instanceof Error ? e.message : "Couldn't update the shopping list.";
    } finally {
      const { [index]: _done, ...rest } = inFlight;
      inFlight = rest;
    }
  }
</script>

<Buy
  status={list.status}
  recipe={list.data}
  error={list.error}
  {ticks}
  {shared}
  {tickError}
  onToggle={toggle}
/>
