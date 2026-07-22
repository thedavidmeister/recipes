<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { replaceState } from "$app/navigation";
  import {
    listKitchens,
    getKitchen,
    createKitchen,
    joinKitchen,
    addEquipment,
    removeEquipment,
    addPantry,
    removePantry,
    currentKitchen,
    stashCurrentKitchen,
    forgetCurrentKitchen,
  } from "$lib/kitchens";
  import type { KitchenDetail, KitchensStatus } from "$lib/types";
  import Kitchens from "$lib/components/Kitchens.svelte";

  /**
   * `kitchens` (#72) — the durable shared space that scopes the meal flow.
   *
   * The page owns the list + the open kitchen's detail (TanStack), and the mutations
   * (create, join, add/remove equipment/pantry) — each returns the kitchen's fresh
   * detail, which is written straight back into the cache so the UI updates without a
   * refetch. `Kitchens` renders. A shareable `?join=<token>` link is redeemed on
   * arrival. All calls are session-gated via `apiFetch` — a kitchen is owned data,
   * not the public corpus.
   */
  const qc = useQueryClient();

  const list = createQuery(() => ({
    queryKey: ["kitchens"],
    queryFn: listKitchens,
    retry: false,
  }));

  let selectedId = $state<string | null>(currentKitchen());

  // Default to the first kitchen once the list loads and nothing is chosen.
  $effect(() => {
    if (!selectedId && list.data && list.data.length > 0) {
      selectedId = list.data[0].id;
    }
  });

  const detail = createQuery(() => ({
    queryKey: ["kitchen", selectedId],
    queryFn: () => getKitchen(selectedId as string),
    enabled: !!selectedId,
    retry: false,
  }));

  const status = $derived<KitchensStatus>(
    list.isError ? "error" : list.isPending ? "pending" : "ready",
  );

  const inviteLink = $derived(
    detail.data && typeof window !== "undefined"
      ? `${window.location.origin}/kitchens?join=${detail.data.invite_token}`
      : undefined,
  );

  /**
   * Why the last thing the user tried didn't happen. Every call into `$lib/kitchens`
   * throws `ApiError` on failure; unhandled, that is an unhandled rejection and the
   * user sees nothing at all. So each action runs through `attempt`: the message lands
   * here for the UI, and the throw carries on so the caller (the input that would
   * otherwise clear itself) knows the action did not land.
   */
  let actionError = $state<string | null>(null);

  async function attempt<T>(run: () => Promise<T>, fallback: string): Promise<T> {
    actionError = null;
    try {
      return await run();
    } catch (e) {
      actionError = e instanceof Error ? e.message : fallback;
      throw e;
    }
  }

  function select(id: string) {
    if (id !== selectedId) actionError = null;
    selectedId = id;
    stashCurrentKitchen(id);
  }

  /** Write a mutation's fresh detail into the cache so the UI updates immediately. */
  function cache(k: KitchenDetail) {
    qc.setQueryData(["kitchen", k.id], k);
  }

  /** Take the kitchen a create/join just returned as the open one. */
  async function adopt(k: KitchenDetail) {
    cache(k);
    select(k.id);
    await qc.invalidateQueries({ queryKey: ["kitchens"] });
  }

  async function onCreate(name: string) {
    await adopt(
      await attempt(() => createKitchen(name), "could not create the kitchen"),
    );
  }

  async function onJoin(token: string) {
    await adopt(await attempt(() => joinKitchen(token), "could not join that kitchen"));
  }

  async function onAddEquipment(item: string) {
    if (!selectedId) return;
    const id = selectedId;
    cache(await attempt(() => addEquipment(id, item), "could not add that equipment"));
  }
  async function onRemoveEquipment(item: string) {
    if (!selectedId) return;
    const id = selectedId;
    cache(
      await attempt(() => removeEquipment(id, item), "could not remove that equipment"),
    );
  }
  async function onAddPantry(item: string) {
    if (!selectedId) return;
    const id = selectedId;
    cache(await attempt(() => addPantry(id, item), "could not add that to the pantry"));
  }
  async function onRemovePantry(item: string) {
    if (!selectedId) return;
    const id = selectedId;
    cache(
      await attempt(
        () => removePantry(id, item),
        "could not remove that from the pantry",
      ),
    );
  }

  // A kitchen that won't open — a 403 after being removed from it, or an id that no
  // longer exists — leaves the detail empty while the list still renders, so without
  // this the page is a bare header. Derived, not pushed into `actionError`: it tracks
  // the query, so nothing the user does next can clear it while the kitchen is still
  // unreachable. The list and the picker stay up: the way out is another kitchen.
  const detailError = $derived(
    detail.isError
      ? detail.error instanceof Error
        ? detail.error.message
        : "could not open this kitchen"
      : null,
  );

  // Forget the remembered id so a reload doesn't land straight back on it.
  $effect(() => {
    if (detail.isError) forgetCurrentKitchen();
  });

  // A shareable invite is a `?join=<token>` link; redeem it once on arrival, then drop
  // the param — otherwise a reload replays a token that is now spent.
  const attempted = new Set<string>();
  $effect(() => {
    const token = page.url.searchParams.get("join");
    if (token && !attempted.has(token)) {
      attempted.add(token);
      onJoin(token)
        .then(() => {
          const url = new URL(page.url);
          url.searchParams.delete("join");
          replaceState(url, page.state);
        })
        .catch(() => {
          // `attempt` already put the reason on screen.
        });
    }
  });
</script>

<Kitchens
  {status}
  {inviteLink}
  kitchens={list.data}
  selected={detail.data}
  error={list.error instanceof Error ? list.error.message : undefined}
  actionError={actionError ?? detailError ?? undefined}
  {onCreate}
  {onJoin}
  onSelect={select}
  {onAddEquipment}
  {onRemoveEquipment}
  {onAddPantry}
  {onRemovePantry}
/>
