<script lang="ts">
  import { onMount } from "svelte";
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { getWalk } from "$lib/walk";
  import { ApiError } from "$lib/client";
  import { DeciderClient, fetchCard, type ConnStatus } from "$lib/session";
  import type {
    RecipeCard,
    SessionStatus,
    WinCondition,
    Winner,
  } from "$lib/types";
  import Decider from "$lib/components/Decider.svelte";
  import Winners from "$lib/components/Winners.svelte";

  /**
   * A live cook-decider session (#20) — the multiplayer mode of `pick`.
   *
   * The page owns the socket, the deck, and the cross-pollination; the components
   * render. Each client walks the corpus **independently** for its starting deck,
   * but every vote (mine or a peer's) arrives over the room and, if it names a
   * recipe I have not queued, that recipe is fetched and slipped silently into my
   * deck — so the group diverges to explore yet converges on every candidate. Turso
   * is the truth: the server re-sends the whole tally on every (re)connect, so a
   * dropped socket just replaces the tally, never loses a vote.
   */
  const channel = $derived(page.params.channel ?? "");
  const queryClient = useQueryClient();

  // The starting deck: a walk over the corpus (its own order for this client).
  const walk = createQuery(() => ({
    queryKey: ["walk", channel],
    queryFn: () => getWalk(),
    staleTime: Infinity,
    retry: false,
  }));

  // ---- session state (reactive so the tally + winners re-derive) ----
  let conn = $state<ConnStatus>("connecting");
  let view = $state<"swipe" | "winners">("swipe");
  let condition = $state<WinCondition>("plurality");
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

  // Fetch a card the tally references but this client has not walked to, so the
  // winners view can render it. Optionally slip it into the deck (peer-injection).
  async function pull(source: string, id: string, toDeck: boolean) {
    const k = key(source, id);
    if (cardMap[k] && !toDeck) return;
    const card = await fetchCard(source, id);
    if (!card) return;
    rememberCard(card);
    if (toDeck) deck = [...deck, card];
  }

  let client: DeciderClient | null = null;

  onMount(() => {
    client = new DeciderClient(channel, {
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
        // Cross-pollinate: a recipe a peer voted, that I have not queued, joins
        // my deck silently.
        if (!queued.has(k)) {
          queued.add(k);
          void pull(source, id, true);
        }
      },
    });
    client.start();
    return () => client?.stop();
  });

  // Seed the deck from the walk once, skipping anything already in play.
  let seeded = false;
  $effect(() => {
    const stops = walk.data;
    if (seeded || !stops) return;
    seeded = true;
    const fresh: RecipeCard[] = [];
    for (const stop of stops) {
      const k = key(stop.recipe.source, stop.recipe.id);
      if (queued.has(k)) continue;
      queued.add(k);
      rememberCard(stop.recipe);
      fresh.push(stop.recipe);
    }
    deck = [...deck, ...fresh];
  });

  // A lapsed session 401s the walk — drop back to login, the only real recovery.
  $effect(() => {
    if (walk.error instanceof ApiError && walk.error.status === 401) {
      queryClient.invalidateQueries({ queryKey: ["session"] });
    }
  });

  const current = $derived(deck[0]);
  const participants = $derived(
    Math.max(serverParticipants, voterIds.length, 1),
  );
  const inTheRunning = $derived(Object.values(yes).filter((c) => c > 0).length);

  const status = $derived<SessionStatus>(
    conn === "connecting"
      ? "connecting"
      : conn === "reconnecting"
        ? "reconnecting"
        : walk.isError && deck.length === 0
          ? "error"
          : walk.isPending && deck.length === 0
            ? "connecting"
            : current
              ? "swiping"
              : "empty",
  );

  const candidates = $derived<Winner[]>(
    Array.from(new Set([...Object.keys(yes), ...Object.keys(no)]))
      .map((k) => {
        const card = cardMap[k];
        return card ? { card, yes: yes[k] ?? 0, no: no[k] ?? 0 } : null;
      })
      .filter((w): w is Winner => w !== null),
  );

  function vote(y: boolean) {
    const c = current;
    if (!c) return;
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

{#if view === "winners"}
  <Winners
    {condition}
    {participants}
    {candidates}
    onCondition={(c) => (condition = c)}
    onBack={() => (view = "swipe")}
  />
{:else}
  <Decider
    {status}
    card={current}
    {inTheRunning}
    {participants}
    error={walk.error instanceof Error ? walk.error.message : undefined}
    shareUrl={page.url.href}
    {copied}
    onVote={vote}
    onShare={share}
    onWinners={() => (view = "winners")}
  />
{/if}
