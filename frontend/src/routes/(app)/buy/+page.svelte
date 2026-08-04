<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { resource } from "$lib/resource";
  import { me } from "$lib/auth";
  import {
    getBuyList,
    getChecks,
    loadChecks,
    saveChecks,
    type BuyCheck,
  } from "$lib/buy";
  import { localTicks, sharedTicks, type Tick } from "$lib/shopping";
  import { PickClient, type Voter } from "$lib/pick";
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
   * **Including this screen's own tick, from the first paint** (#210). A tap needs no
   * round trip to know whose it is: the session this page is holding says so, so the
   * row goes on wearing the tapper's colour and name at once and the announcement
   * confirms it rather than restyling it. The fold that does it is `sharedTicks` in
   * `$lib/shopping`, where a test can reach it.
   *
   * Since #209 a tick is **an event on that same room's socket**, not a `POST` beside
   * it: one path in for every session write (`$lib/session-events`), carrying the
   * instant this device tapped rather than the instant a row happened to be written.
   * So the socket below is no longer only how a peer's tick arrives — it is how this
   * screen's own tick leaves, and the `buy` frame that comes back is the one answer
   * both of them land on.
   *
   * The exception is a decision with no session behind it: nothing to write to and no
   * room to announce into, so the list falls back to this device's own
   * (`loadChecks`/`saveChecks`) and says so on screen. Ticking is never refused over
   * the lack of a group — a shopping list you cannot tick is not a shopping list. No
   * group is still not "nobody", though: a tap made here is the person at this screen's
   * and is theirs from the first paint too, while what storage handed back on load is
   * the one tick left with genuinely nobody behind it (`localTicks`).
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

  /**
   * Who is doing the shopping — the one fact that lets a tap be attributed before the
   * room has answered for it (#210).
   *
   * The layout has already fetched the session (the whole `(app)` group is behind its
   * gate, so nothing under it renders for nobody), which makes this a cache read rather
   * than a request — the same way `cook` asks who is watching. Narrowed to a `Voter`
   * because that is what a ticked line carries: the same two fields the server puts on
   * `by`, so an optimistic tick and the announced one are the same shape and the
   * confirmation changes nothing on screen.
   */
  const session = createQuery(() => ({ queryKey: ["session"], queryFn: me }));
  const you = $derived<Voter | null>(
    session.data
      ? {
          telegram_user_id: session.data.telegram_user_id,
          username: session.data.username,
        }
      : null,
  );

  // The shared list as the server last stated it — replaced whole on every answer
  // and every room announcement, never merged: a tick can take an item off somebody
  // else (last writer wins), so a delta would be a lie.
  let checks = $state<BuyCheck[]>([]);
  // Taps that have not been confirmed yet: index → the state this client asked for.
  // Kept beside the server's answer rather than folded into it, so the room's next
  // whole-list frame replaces one and clears the other, and a tap it refused has exactly
  // one place to disappear from — colour and all (#210).
  let inFlight = $state<Record<number, boolean>>({});
  // The plan's room. Held here rather than only inside the effect that opens it,
  // because a tap raises an event on it (#209) — the socket is this page's way out as
  // well as its way in.
  let client = $state<PickClient | null>(null);
  // The device-local list, for a decision with no session to attribute to.
  let localChecked = $state<Record<number, true>>({});
  // Which of those this person ticked in *this* sitting. The rest came out of storage,
  // which outlives a sitting and is shared with whoever else uses this browser — so
  // they are the one remaining tick nobody can be named for (#210). Reset with the list.
  let localMine = $state<Record<number, true>>({});
  // Why the shared list could not be opened, when it could not be. Since #209 that is
  // the only thing it ever carries: reading the list is still an answerable request, so
  // its failure still has a sentence, while a *tick* is an event on a socket the server
  // never answers a frame on — a refusal there is silent (#179/#180) and what the screen
  // is told instead is the truth, in the room's next whole-list frame.
  let tickError = $state<string | undefined>();

  /**
   * What the screen shows: where each ticked line's tick came from.
   *
   * The **only** place the two paths meet, and neither knows about the other — `cook`'s
   * arrangement with its timers, and for the same reason. Both folds are pure and live
   * in `$lib/shopping`, where a unit test can reach them; this page's job is to say
   * which one is in play and hand it the three things it holds.
   *
   * A pantry pre-tick (#156) is passed through by both untouched: it is nobody's, but
   * it is not the same nobody as an unattributed tick, and it carries the entry that
   * answered for the line so the row can say *why* it is ticked.
   */
  const ticks = $derived<Record<number, Tick>>(
    shared
      ? sharedTicks(checks, inFlight, you)
      : localTicks(localChecked, localMine, you),
  );

  // Load this recipe's ticks when it arrives (or changes). The two paths are read
  // the same way and differ only in where from.
  $effect(() => {
    const r = list.data;
    checks = [];
    inFlight = {};
    localChecked = {};
    localMine = {};
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

  // The meal's room: every tick's way out and every tick's way back, this screen's
  // and everybody else's — the same socket the pick uses, listening only for the
  // frame this page is about.
  $effect(() => {
    const r = list.data;
    if (!r?.channel) return;
    const c = new PickClient(r.channel, {
      onBuy: (source, id, incoming) => {
        // Another recipe's list in the same meal is not this screen's business.
        if (source !== r.source || id !== r.id) return;
        checks = incoming;
        // The room has stated the truth about this list, so nothing this client asked
        // for is still in flight against it. The server sends the **whole** list on
        // every tick it takes, including one its own predicate refused, so this is
        // also how a refused tap goes back to what is actually in the basket.
        inFlight = {};
      },
      onStatus: (s) => {
        // A tap is only optimistic while a connection exists that could answer for it.
        // When the socket drops, the answer this device was waiting on is not coming —
        // the room re-sends its list on a tick, never on a connect — so the taps go
        // with it and the screen falls back to the last thing the server actually said
        // (#210). A row that wrongly shows *not got* costs a second tap; one that
        // wrongly shows got, in somebody's colour, is how they come home without the
        // flour, and that asymmetry is already this page's rule.
        if (s !== "open") inFlight = {};
      },
    });
    c.start();
    client = c;
    return () => {
      c.stop();
      client = null;
    };
  });

  /**
   * Tick or untick a line.
   *
   * Optimistic, then confirmed: a tap has to feel like a tap in a supermarket aisle,
   * so the row moves at once — **as the tapper's, colour and name** (#210) — and the
   * round trip catches up. What confirms it is the room's own `buy` frame — the whole
   * list, sent to everybody shopping this meal on every tick anyone takes — so this
   * client's answer and its neighbour's are the same message, and a tap the server
   * refused comes back as the basket that actually exists. A line that looks got but is
   * not is how somebody comes home without the flour, so the truth always wins over the
   * optimism; the attribution is part of the optimism and goes back with it.
   *
   * A tap with no open socket is dropped, exactly as a swipe is (`PickClient.event`):
   * the durable record is the server's, and a tap that never left is a tap to make
   * again. It is deliberately not queued for the reconnect — a tick replayed minutes
   * later would claim a line somebody has since put in their own basket. Since the row
   * now moves in somebody's name, a dropped tap must not move it at all: an unanswerable
   * claim is worse in a colour than it was in plain stone.
   */
  function toggle(index: number) {
    const r = list.data;
    if (!r) return;

    if (!r.channel) {
      const next = { ...localChecked };
      // The same tap, recorded twice: what is ticked (which is persisted, and outlives
      // this sitting) and that *this* person ticked it (which does not, because storage
      // cannot say who wrote it). Unticking drops both — a line with no tick has nobody.
      const mine = { ...localMine };
      if (next[index]) {
        delete next[index];
        delete mine[index];
      } else {
        next[index] = true;
        mine[index] = true;
      }
      localChecked = next;
      localMine = mine;
      saveChecks(r.source, r.id, Object.keys(next).map(Number));
      return;
    }

    const want = !(index in ticks);
    // Paint ahead of the answer only when there is going to be one. `tick` says whether
    // the frame reached a socket (`PickClient.event`), and a tap that never left has
    // neither a confirmation nor a refusal coming — so it gets no optimism to strand,
    // and no colour with it (#210). The row simply does not move, which is the honest
    // report of a tap that did nothing and an invitation to make it again.
    if (!client?.tick(r.source, r.id, index, want)) return;
    inFlight = { ...inFlight, [index]: want };
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
