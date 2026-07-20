<script lang="ts">
  import { onMount } from "svelte";
  import { useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { getWalk } from "$lib/walk";
  import { ApiError } from "$lib/client";
  import { PickClient, fetchCard, type ConnStatus } from "$lib/pick";
  import type { Match, PickStatus, RecipeCard } from "$lib/types";
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

  // ---- pick state (reactive so the tally + matches re-derive) ----
  let conn = $state<ConnStatus>("connecting");
  let copied = $state(false);

  let deck = $state<RecipeCard[]>([]); // my swipe queue
  let cardMap = $state<Record<string, RecipeCard>>({}); // key -> card
  let yes = $state<Record<string, number>>({}); // key -> yes count
  let no = $state<Record<string, number>>({}); // key -> no count
  let voterIds = $state<string[]>([]); // distinct voters seen live
  let serverParticipants = $state(0); // authoritative count from the last tally

  // Dedupe only (never rendered), so a plain Set is fine.
  const queued = new Set<string>();

  const key = (s: string, i: string) => `${s}:${i}`;

  function rememberCard(card: RecipeCard) {
    const k = key(card.source, card.id);
    if (!cardMap[k]) cardMap = { ...cardMap, [k]: card };
  }

  // Fetch a card the tally references but this client has not walked to, so a match
  // can render it. Optionally slip it into the deck (peer-injection).
  async function pull(source: string, id: string, toDeck: boolean) {
    const k = key(source, id);
    if (cardMap[k] && !toDeck) return;
    const card = await fetchCard(source, id);
    if (!card) return;
    rememberCard(card);
    if (toDeck) deck = [...deck, card];
  }

  // ---- the endless deck ----
  // A pick never runs dry: prefetch well before the last card, and size the buffer
  // to the swiper — ~2x their recent swipes-per-minute — so a fast swiper is fed a
  // deeper queue and a browser a shallow one, and "Finding more…" is a rare bridge.
  let refilling = $state(false);
  let loadedOnce = $state(false);
  let dry = $state(false); // nothing fresh right now — back off, don't busy-loop

  // Recent swipe times (plain — logic only, never rendered) → a live rate.
  const swipeTimes: number[] = [];
  let spm = $state(12); // swipes/minute; a modest default until we have a rate

  function recordSwipe() {
    const now = Date.now();
    swipeTimes.push(now);
    while (swipeTimes.length && now - swipeTimes[0] >= 90_000) swipeTimes.shift();
    if (swipeTimes.length >= 3) {
      const spanMin =
        (swipeTimes[swipeTimes.length - 1] - swipeTimes[0]) / 60_000;
      if (spanMin > 0) spm = (swipeTimes.length - 1) / spanMin;
    }
  }

  // How many cards to keep ahead of the swiper: 2x their rate, bounded. A walk
  // yields at most MAX_LEN (30) per call, so a deeper buffer just costs one more.
  const bufferTarget = $derived(Math.min(40, Math.max(10, Math.round(2 * spm))));

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
      for (let fetches = 0; deck.length < bufferTarget && fetches < 3; fetches++) {
        const stops = await getWalk(30);
        const fresh: RecipeCard[] = [];
        for (const s of stops) {
          const k = key(s.recipe.source, s.recipe.id);
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

  // Prefetch before the deck runs low, sized to the swiper — the buffer stays
  // ahead of the swiping so the next card is always ready.
  $effect(() => {
    if (deck.length < bufferTarget && !refilling && !dry) void refill();
  });

  let client: PickClient | null = null;

  onMount(() => {
    client = new PickClient(channel, {
      onStatus: (s) => (conn = s),
      onTally: (participants, votes) => {
        serverParticipants = participants;
        const y: Record<string, number> = {};
        const n: Record<string, number> = {};
        for (const v of votes) {
          const k = key(v.source, v.id);
          y[k] = v.yes;
          n[k] = v.no;
          if (!cardMap[k]) void pull(v.source, v.id, false);
        }
        yes = y;
        no = n;
      },
      onVote: (voter, source, id, vote) => {
        if (!voterIds.includes(voter)) voterIds = [...voterIds, voter];
        const k = key(source, id);
        if (vote) yes = { ...yes, [k]: (yes[k] ?? 0) + 1 };
        else no = { ...no, [k]: (no[k] ?? 0) + 1 };
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
  const participants = $derived(
    Math.max(serverParticipants, voterIds.length, 1),
  );

  // The pick: recipes everyone in the room said yes to. Consensus needs a group —
  // a solo swiper has no match until someone else joins and agrees.
  const matches = $derived<Match[]>(
    participants < 2
      ? []
      : Object.keys(yes)
          .filter((k) => (yes[k] ?? 0) === participants && (no[k] ?? 0) === 0)
          .map((k) => {
            const card = cardMap[k];
            return card ? { card, yes: yes[k] ?? 0 } : null;
          })
          .filter((m): m is Match => m !== null),
  );

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
    recordSwipe();
    queued.add(key(c.source, c.id));
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

<Pick
  {status}
  card={current}
  {matches}
  {participants}
  shareUrl={page.url.href}
  {copied}
  onVote={vote}
  onShare={share}
/>
