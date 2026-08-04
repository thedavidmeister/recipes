<script lang="ts">
  import { onMount } from "svelte";
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { getWalk } from "$lib/walk";
  import { answeredEverything } from "$lib/deal";
  import { ApiError } from "$lib/client";
  import {
    PickClient,
    fetchCard,
    getLobby,
    joinLobby,
    leavePlan,
    startPlan,
    seatMember,
    setAdditions,
    setMealType,
    setPlanCap,
    setPlanCalories,
    type ConnStatus,
    type Decided,
    type Lobby,
    type Voter,
  } from "$lib/pick";
  import PlanLobby from "$lib/components/PlanLobby.svelte";
  import { isWatching } from "$lib/roster";
  import { cardKey, decidingCount } from "$lib/consensus";
  import { me } from "$lib/auth";
  import { stashConsensus } from "$lib/buy";
  import type {
    MealAddition,
    MealType,
    PickStatus,
    RecipeCard,
  } from "$lib/types";
  import Pick from "$lib/components/Pick.svelte";

  /**
   * A pick (#20): an endless, shared swipe over the corpus, focused on **consensus**.
   *
   * The page owns the socket, the deck, and the cross-pollination; `Pick` renders.
   * Each client walks the corpus **independently** for its deck, which **refills
   * endlessly** — a pick never runs out until the group finds a **match** (a recipe
   * everyone said yes to). Every vote (mine or a peer's) arrives over the room and,
   * if it names a recipe I have not queued, is fetched and slipped silently into my
   * deck — so the pick diverges to explore yet converges on every candidate. Turso
   * is the truth: the server re-sends the whole tally on every (re)connect, so a
   * dropped socket just replaces the tally, never loses a vote. This URL is the
   * shareable invite.
   */
  const channel = $derived(page.params.channel ?? "");
  const queryClient = useQueryClient();

  // ---- pick state (reactive so the tally + consensus re-derive) ----
  let conn = $state<ConnStatus>("connecting");
  let copied = $state(false);

  let deck = $state<RecipeCard[]>([]); // my swipe queue
  let cardMap = $state<Record<string, RecipeCard>>({}); // key -> card
  let yes = $state<Record<string, number>>({}); // key -> yes count
  let no = $state<Record<string, number>>({}); // key -> no count
  // key -> the telegram ids that said yes, so a card can wear the colours of the
  // people who already like it (#131/#145). The tally carries these too, not just
  // the live frames, so a reconnect rehydrates the attribution rather than losing
  // it with the socket.
  let yesIds = $state<Record<string, string[]>>({});
  let voterIds = $state<string[]>([]); // distinct voters seen live
  // The lobby roster size — who a recipe has to win over — as the server last stated
  // it, and `undefined` until it has (#181). It comes from the `lobby` frame, which
  // the server sends on connect and again on every roster change, so this is the
  // server's count rather than one the client kept; the lobby read on mount seeds the
  // same number from the same place.
  //
  // Since #201 it is what the footer *shows*, not what anything here measures against:
  // the server holds this roster and evaluates the win condition inside the vote's own
  // write. The unknown-is-not-one care below is still worth keeping — a caption that
  // says "1 deciding" to a room of three is still wrong — but a wrong number here can
  // no longer end a pick.
  let deciders = $state<number | undefined>();
  let started = $state<boolean | undefined>(); // undefined until the lobby is known
  let lobby = $state<Lobby | undefined>();
  let lobbyError = $state<string | undefined>();
  // The last person walked out, so this plan is gone (#96). Only ever reachable
  // while the lobby is open — a plan can only be emptied before it starts — so it
  // is the lobby that says so, and it outranks everything else it could say.
  let planEnded = $state(false);

  // Dedupe only (never rendered), so plain Sets are fine. `queued` guards the deck
  // (a recipe is queued once); `pulling` guards in-flight card fetches so a failing
  // fetch is not re-issued on every tally frame.
  const queued = new Set<string>();
  const pulling = new Set<string>();

  function rememberCard(card: RecipeCard) {
    const k = cardKey(card.source, card.id);
    if (!cardMap[k]) cardMap = { ...cardMap, [k]: card };
  }

  // Fetch a card the tally references but this client has not walked to, so a match
  // can render it. Optionally slip it into the deck (peer-injection).
  async function pull(source: string, id: string, toDeck: boolean) {
    const k = cardKey(source, id);
    if (cardMap[k] && !toDeck) return;
    if (pulling.has(k)) return; // one fetch in flight per key
    pulling.add(k);
    try {
      const card = await fetchCard(source, id);
      if (!card) return;
      rememberCard(card);
      if (toDeck) deck = [...deck, card];
    } catch {
      // A Turso read failed; leave the card unresolved so a later tally/vote can
      // retry, instead of crashing on an unhandled rejection.
    } finally {
      pulling.delete(k);
    }
  }

  // ---- the endless deck ----
  // A pick never runs dry: prefetch well before the last card, and size the buffer
  // to the swiper — ~2x their recent swipes-per-minute — so a fast swiper is fed a
  // deeper queue and a browser a shallow one, and "Finding more…" is a rare bridge.
  let refilling = $state(false);
  let loadedOnce = $state(false);
  let dry = $state(false); // nothing fresh right now — back off, don't busy-loop
  // …except a pick *can* run dry, and since #202 it says so instead of hunting. The
  // deal skips what this member has already voted on in this plan, so an empty deal
  // means they have answered everything the plan can currently serve them and are
  // waiting on the others. Read off every deal rather than latched, so a recipe
  // becoming dealable mid-plan (the meal-time worker reading one this round can serve)
  // un-finishes it on the very next refill with nothing to invalidate.
  let finished = $state(false);

  // Recent swipe times (plain — logic only, never rendered) → a live rate.
  const swipeTimes: number[] = [];
  let spm = $state(12); // swipes/minute; a modest default until we have a rate

  function recordSwipe() {
    const now = Date.now();
    swipeTimes.push(now);
    while (swipeTimes.length && now - swipeTimes[0] >= 90_000)
      swipeTimes.shift();
    if (swipeTimes.length >= 3) {
      const spanMin =
        (swipeTimes[swipeTimes.length - 1] - swipeTimes[0]) / 60_000;
      if (spanMin > 0) spm = (swipeTimes.length - 1) / spanMin;
    }
  }

  // How many cards to keep ahead of the swiper: 2x their rate, bounded. A walk
  // yields at most MAX_LEN (30) per call, so a deeper buffer just costs one more.
  const bufferTarget = $derived(
    Math.min(40, Math.max(10, Math.round(2 * spm))),
  );

  function backoff() {
    dry = true;
    setTimeout(() => (dry = false), 3000);
  }

  async function refill() {
    if (refilling) return;
    refilling = true;
    try {
      let added = false;
      // Top up toward the buffer target. A walk is a different journey each call,
      // so a couple of fetches surface fresh cards even as `queued` grows.
      for (
        let fetches = 0;
        deck.length < bufferTarget && fetches < 3;
        fetches++
      ) {
        // The channel travels with the walk so the server bounds it to the plan's
        // time cap (#80) — the cap itself never comes from the client. It is also how
        // the server knows whose round this is, and so what this member has already
        // answered in it (#202); the id comes from the session, never from here.
        const stops = await getWalk(30, channel);
        // What the *deal* held, not what was new to this deck — a walk of cards this
        // client already queued is a client still holding cards, and says nothing about
        // whether the member has answered them.
        finished = answeredEverything(stops.length);
        const fresh: RecipeCard[] = [];
        for (const s of stops) {
          const k = cardKey(s.recipe.source, s.recipe.id);
          if (queued.has(k)) continue;
          queued.add(k);
          rememberCard(s.recipe);
          fresh.push(s.recipe);
        }
        if (!fresh.length) break; // this walk surfaced nothing new
        deck = [...deck, ...fresh];
        added = true;
      }
      loadedOnce = true;
      if (!added) backoff();
    } catch (e) {
      // `finished` is deliberately left alone: a deal that failed said nothing about
      // whether anything is left, so the last one that answered stands until another
      // does. Clearing it here would flash "Finding more recipes…" at the one person
      // for whom that is untrue.
      if (e instanceof ApiError && e.status === 401) {
        // A lapsed session — drop back to login, the only real recovery.
        queryClient.invalidateQueries({ queryKey: ["session"] });
      } else {
        backoff();
      }
    } finally {
      refilling = false;
    }
  }

  /**
   * The pick's decision, **as the server recorded it** (#201).
   *
   * This page used to work the win condition out for itself — `yes === deciders && no
   * === 0` over the rehydrated tally — and then stash the winner in `localStorage`. It
   * does not any more, and the computation is gone rather than kept as a preview: two
   * evaluators of one win condition are two answers to "what did we pick", and the one
   * that holds the roster and the votes is the server. What arrives here is a fact
   * (`ServerMsg::Decided`, or the same record on the lobby read), and this page's job
   * is to act on it.
   *
   * That is also what makes it reach a client that was not watching. A member whose
   * browser was closed when the last yes landed used to have nothing to come back to,
   * because "what we decided" lived only in the browsers that were open. Now the socket
   * hands them the record on connect, and this moves them the same as everybody else.
   *
   * Sticky, as it always was: the plan decided once, and a decision has no undo.
   */
  let decided = $state<Decided | undefined>();

  // Prefetch before the deck runs low, sized to the swiper — the buffer stays ahead
  // of the swiping so the next card is always ready. Stops once the pick is decided.
  //
  // The deck only builds once the plan has *started*: while the lobby is open the
  // host can still be moving the time cap (#80), so a deck fetched earlier could
  // hold cards outside the bound everyone agreed to swipe within. Start freezes
  // the cap; only then is a card worth dealing.
  $effect(() => {
    if (
      started === true &&
      deck.length < bufferTarget &&
      !refilling &&
      !dry &&
      !decided
    )
      void refill();
  });

  /**
   * Arriving *is* joining. The URL is the invite, so opening it seats you — there is
   * no accept step, because there is nothing to accept: you followed the link.
   *
   * Once the swiping has begun the server refuses a newcomer, and that refusal is the
   * lobby's whole guarantee: the number a recipe had to win over cannot move under
   * people who are already voting.
   */
  async function refreshLobby() {
    try {
      lobby =
        started === false || started === undefined
          ? await joinLobby(channel)
          : await getLobby(channel);
      deciders = lobby.voters.length;
      started = lobby.started;
      // The lobby carries what the plan decided (#201), so a page that has read it
      // knows the pick is over without waiting for its socket. Same record the
      // `decided` frame carries, off the same row, so the two cannot disagree —
      // whichever arrives first moves this client and the other is a no-op.
      decided = lobby.decided ?? decided;
      lobbyError = undefined;
    } catch (e) {
      // Already started and not on the roster: the join is refused, the plain read is
      // not, and that read is the point — it carries the roster `watching` below
      // measures this viewer against (#180). Watching is a state this page renders,
      // so a refused seat is something to go and find out about, not an error.
      try {
        lobby = await getLobby(channel);
        deciders = lobby.voters.length;
        started = lobby.started;
        decided = lobby.decided ?? decided;
      } catch {
        lobbyError =
          e instanceof Error ? e.message : "Couldn't open this meal plan.";
      }
    }
  }

  async function begin() {
    try {
      lobby = await startPlan(channel);
      started = lobby.started;
      deciders = lobby.voters.length;
    } catch (e) {
      lobbyError =
        e instanceof Error ? e.message : "Couldn't start this meal plan.";
    }
  }

  /**
   * Step out of the plan (#96) — a lobby act, because that is the only place the
   * roster moves at all: it closes at the start in both directions, so the server
   * refuses this once the swiping has begun.
   *
   * The socket is closed *before* the request lands, so a client on its way out is
   * not still holding a room it is being removed from, and then it goes back to the
   * kitchen the plan was called in — or to `/`, which resolves your own.
   */
  async function leave() {
    try {
      client?.stop();
      const gone = await leavePlan(channel);
      await goto(gone.kitchen_id ? `/kitchens/${gone.kitchen_id}` : "/");
    } catch (e) {
      // Still in the plan, so the room is still worth listening to.
      client?.start();
      lobbyError =
        e instanceof Error ? e.message : "Couldn't leave this meal plan.";
    }
  }

  async function seat(userId: string) {
    try {
      lobby = await seatMember(channel, userId);
      deciders = lobby.voters.length;
    } catch (e) {
      lobbyError = e instanceof Error ? e.message : "Couldn't add that person.";
    }
  }

  /** The host names which meal this plans (#114); the room announcement re-reads
   * the lobby on every other open client, so the whole roster sees it. */
  async function chooseMeal(mealType: MealType) {
    try {
      lobby = await setMealType(channel, mealType);
    } catch (e) {
      lobbyError = e instanceof Error ? e.message : "Couldn't change the meal.";
    }
  }

  /** The host names what comes with the meal (#114) — the whole chosen set. */
  async function chooseAdditions(additions: MealAddition[]) {
    try {
      lobby = await setAdditions(channel, additions);
    } catch (e) {
      lobbyError =
        e instanceof Error ? e.message : "Couldn't change the additions.";
    }
  }

  /** The host bounds the plan to the time the group has (#80); frozen at start. */
  async function capPlan(cap: number | null) {
    try {
      lobby = await setPlanCap(channel, cap);
      deciders = lobby.voters.length;
      started = lobby.started;
    } catch (e) {
      lobbyError =
        e instanceof Error ? e.message : "Couldn't set the time cap.";
    }
  }

  /** The host bounds the plan to how big a serving it is planning (#213); frozen at
   * start, like the cap. Both ends go together — a range is one setting. */
  async function caloriesPlan(min: number | null, max: number | null) {
    try {
      lobby = await setPlanCalories(channel, min, max);
      deciders = lobby.voters.length;
      started = lobby.started;
    } catch (e) {
      lobbyError =
        e instanceof Error ? e.message : "Couldn't set the calorie range.";
    }
  }

  // Who is looking, so the lobby knows whether to offer the start. The layout has
  // already fetched this, so it is a cache read rather than a request.
  const session = createQuery(() => ({
    queryKey: ["session"],
    queryFn: me,
  }));

  /**
   * Watching, not deciding (#180) — the plan started and this viewer has no seat.
   *
   * Derived from the roster rather than latched when the join was refused, because a
   * flag set once goes stale and the roster does not: it is re-read on every lobby
   * announcement, so this answer is only ever as old as the last one. It is also what
   * keeps the two states apart. **A seat is what is asked, never when somebody
   * arrived** — a kitchen member the host seated before the start (#72) who opens the
   * link an hour in is on the roster and votes, which is exactly what `join_lobby`
   * says by re-seating them (its refusal is `started` *and* off the roster).
   *
   * `isWatching` holds the three-way guard and its reasoning; it is a pure module so
   * a story and a unit test can both reach it (`lint:env`).
   */
  const watching = $derived(
    isWatching({
      started,
      roster: lobby?.voters,
      viewer: session.data?.telegram_user_id,
    }),
  );

  let client: PickClient | null = null;

  onMount(() => {
    void refreshLobby();
    client = new PickClient(channel, {
      onStatus: (s) => (conn = s),
      onLobby: (count, begun) => {
        deciders = count;
        started = begun;
        // The roster changed under us — re-read it so the lobby list matches the
        // number it is about to be measured against.
        if (!begun) void refreshLobby();
      },
      onLeft: (_who, ended) => {
        // Who left is not rendered: a lobby announces neither arrivals nor
        // departures, it shows who is here, and the roster frame beside this one
        // already re-reads that list. What has no other frame to carry it is the
        // plan being gone — nobody is left to send a smaller roster — so that is
        // what this handler is for. Stop listening to a room that no longer
        // exists rather than leaving a socket to reconnect at nothing.
        if (ended) {
          planEnded = true;
          client?.stop();
        }
      },
      // The plan decided (#201). The only thing that ends a pick, and it is a fact
      // rather than a conclusion this client reached: it arrives when the deciding
      // vote lands, and again on every connect, so a member who was offline for the
      // last yes is *told* rather than left to re-derive. Set once — the record is
      // immutable, so a re-send on a reconnect names the same recipe.
      onDecided: (d) => {
        decided ??= d;
      },
      // A tally's own count is how many people have swiped **at all**, not how many
      // are deciding, so it was never what consensus was measured against — the
      // roster is (#181), and since #201 neither of them is measured here at all.
      // Ignored on purpose, rather than kept in a variable that would only ever be
      // the wrong number to reach for.
      onTally: (_participants, votes) => {
        const y: Record<string, number> = {};
        const n: Record<string, number> = {};
        const who: Record<string, string[]> = {};
        for (const v of votes) {
          const k = cardKey(v.source, v.id);
          y[k] = v.yes;
          n[k] = v.no;
          who[k] = v.yes_voters;
          if (!cardMap[k]) void pull(v.source, v.id, false);
        }
        yes = y;
        no = n;
        yesIds = who;
      },
      onVote: (voter, source, id, vote) => {
        if (!voterIds.includes(voter)) voterIds = [...voterIds, voter];
        const k = cardKey(source, id);
        if (vote) yes = { ...yes, [k]: (yes[k] ?? 0) + 1 };
        else no = { ...no, [k]: (no[k] ?? 0) + 1 };
        // A vote is a current call, not an append (`record_vote`), so a no takes
        // its author back off the yes list rather than leaving a stale colour on
        // the card.
        const had = yesIds[k] ?? [];
        yesIds = {
          ...yesIds,
          [k]: vote
            ? had.includes(voter)
              ? had
              : [...had, voter]
            : had.filter((v) => v !== voter),
        };
        // Cross-pollinate: a recipe a peer voted, that I have not queued, joins my
        // deck silently.
        if (!queued.has(k)) {
          queued.add(k);
          void pull(source, id, true);
        }
      },
    });
    client.start();
    return () => client?.stop();
  });

  const current = $derived(deck[0]);

  /**
   * Who has already said yes to the card on screen (#131/#145).
   *
   * A card mostly arrives here *because* a peer voted it — that is what
   * cross-pollination is — so "Mel already likes this" is the context that makes
   * the swipe a conversation rather than a solo sort. Names come from the roster,
   * which is the only place a handle lives; someone the roster does not carry
   * still shows, by id, because a vote is never withheld over a missing handle.
   */
  const yesVoters = $derived<Voter[]>(
    (current ? (yesIds[cardKey(current.source, current.id)] ?? []) : []).map(
      (id) =>
        lobby?.voters.find((v) => v.telegram_user_id === id) ?? {
          telegram_user_id: id,
          username: null,
        },
    ),
  );
  /**
   * How many people a recipe has to win over: the lobby roster, and `undefined` until
   * the server has stated it.
   *
   * This is the number the lobby exists to establish. Inferring it was the old bug in
   * every direction (#181) — counting who had voted meant one person's first yes was
   * already unanimous, and counting who was connected meant a reload looked like
   * somebody leaving. You are deciding because you joined, and you keep deciding while
   * you make a cup of tea.
   *
   * **Display only, since #201.** The count this page shows and the count a pick is
   * decided against were the same number and are not any more: the server holds the
   * roster and evaluates the win condition inside the vote's own write. So a wrong
   * number here is now a wrong caption rather than a group sent shopping for a recipe
   * it never agreed on — which is why `agreed` is gone from this page and
   * `decidingCount` is not: the floor-of-one and the unknown-is-not-one rule still
   * decide what the footer says, and both still deserve `$lib/consensus`'s tests.
   */
  const deciding = $derived(decidingCount(deciders));

  /**
   * Move everyone the moment the plan decides (#201) — including the person whose
   * browser was closed when it happened.
   *
   * The one thing that ends a pick, and it is the server's record: either the live
   * `decided` frame, or the same record on the lobby read for a page that has not
   * finished rehydrating. **There is no client-side win condition left to disagree with
   * it** — #197's `agreed` was the last one, and it is not called here any more, because
   * two evaluators of one condition are two answers to "what did we pick" and the
   * server is the one holding the roster and the votes.
   *
   * Both sources name the same recipe, so whichever lands first moves this client and
   * `leaving` makes the other a no-op — a plain `let`, like `queued` and `pulling`,
   * because it dedupes and is never rendered, and a `$state` here would put the effect
   * back into its own dependencies.
   *
   * The card may be one this client never walked to, which is the whole offline case,
   * so it is fetched if it is not already held. A title that cannot be fetched is not
   * worth blocking on: `getBuyList` reads the recipe straight from Turso and only falls
   * back to the stashed title when the corpus has no such row, in which case there is
   * no shopping list to show either.
   */
  let leaving = false;
  $effect(() => {
    const d = decided;
    if (!d || leaving) return;
    leaving = true;
    void (async () => {
      const k = cardKey(d.source, d.id);
      if (!cardMap[k]) await pull(d.source, d.id, false);
      stashConsensus({
        source: d.source,
        id: d.id,
        title: cardMap[k]?.title ?? "",
        // The meal travels with the decision, so `buy`'s checklist lands in the
        // session the recipe was agreed in rather than in this browser (#131) — and
        // is now the session that *holds* the decision the list is for.
        channel,
      });
      await goto("/buy");
    })();
  });

  const status = $derived<PickStatus>(
    conn === "reconnecting"
      ? "reconnecting"
      : current
        ? "swiping"
        : conn === "connecting" || !loadedOnce
          ? "connecting"
          : "loading",
  );

  function vote(y: boolean) {
    const c = current;
    if (!c) return;
    // A watcher has no buttons to press, and this is the same sentence said where the
    // frame would be sent: `record_vote` refuses the write and the socket has nowhere
    // to answer, so a vote from here would be swallowed in silence rather than
    // refused out loud — and it would still take the card off this client's deck,
    // which is a change nothing undoes.
    if (watching) return;
    recordSwipe();
    queued.add(cardKey(c.source, c.id));
    client?.vote(c.source, c.id, y); // the echoed vote updates the tally
    deck = deck.slice(1);
  }

  async function share() {
    try {
      await navigator.clipboard.writeText(page.url.href);
      copied = true;
    } catch {
      copied = false;
    }
  }
</script>

{#if started === true}
  <!-- The roster and `started` always come off the same lobby read, so a swipe view
       is never rendered without a count to show — `Pick`'s own default covers the
       case the types cannot rule out. -->
  <Pick
    {status}
    card={current}
    participants={deciding}
    {yesVoters}
    {watching}
    roster={lobby?.voters ?? []}
    {finished}
    shareUrl={page.url.href}
    {copied}
    onVote={vote}
    onShare={share}
  />
{:else}
  <!-- Until the host begins, this is the lobby: the plan exists, the roster is still
       forming, and nothing is being decided yet. It is also where a plan can end —
       the roster only moves before the start (#96), so the last person out empties
       it here, and "this plan is over" outranks anything else the lobby could say:
       every read of a channel that no longer exists 400s, which would otherwise
       show as "couldn't open this meal plan" and send someone hunting a fault. -->
  <PlanLobby
    status={planEnded
      ? "ended"
      : lobbyError
        ? "error"
        : lobby
          ? "ready"
          : "pending"}
    voters={lobby?.voters}
    candidates={lobby?.candidates}
    mealType={lobby?.meal_type}
    additions={lobby?.additions}
    cap={lobby?.max_total_seconds ?? null}
    minKcal={lobby?.min_kcal_per_serving ?? null}
    maxKcal={lobby?.max_kcal_per_serving ?? null}
    host={!!lobby && lobby.host === session.data?.telegram_user_id}
    hostId={lobby?.host}
    inviteLink={page.url.href}
    error={lobbyError}
    onStart={begin}
    onSeat={seat}
    onMealType={chooseMeal}
    onAdditions={chooseAdditions}
    onCap={capPlan}
    onCalories={caloriesPlan}
    onLeave={leave}
  />
{/if}
