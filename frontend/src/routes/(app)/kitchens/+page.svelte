<script lang="ts">
  import { createQuery, useQueryClient } from "@tanstack/svelte-query";
  import { page } from "$app/state";
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

  function select(id: string) {
    selectedId = id;
    stashCurrentKitchen(id);
  }

  /** Write a mutation's fresh detail into the cache so the UI updates immediately. */
  function cache(k: KitchenDetail) {
    qc.setQueryData(["kitchen", k.id], k);
  }

  async function onCreate(name: string) {
    const k = await createKitchen(name);
    cache(k);
    select(k.id);
    await qc.invalidateQueries({ queryKey: ["kitchens"] });
  }

  async function onJoin(token: string) {
    const k = await joinKitchen(token);
    cache(k);
    select(k.id);
    await qc.invalidateQueries({ queryKey: ["kitchens"] });
  }

  async function onAddEquipment(item: string) {
    if (selectedId) cache(await addEquipment(selectedId, item));
  }
  async function onRemoveEquipment(item: string) {
    if (selectedId) cache(await removeEquipment(selectedId, item));
  }
  async function onAddPantry(item: string) {
    if (selectedId) cache(await addPantry(selectedId, item));
  }
  async function onRemovePantry(item: string) {
    if (selectedId) cache(await removePantry(selectedId, item));
  }

  // A shareable invite is a `?join=<token>` link; redeem it once on arrival.
  const attempted = new Set<string>();
  $effect(() => {
    const token = page.url.searchParams.get("join");
    if (token && !attempted.has(token)) {
      attempted.add(token);
      void onJoin(token);
    }
  });
</script>

<Kitchens
  {status}
  {inviteLink}
  kitchens={list.data}
  selected={detail.data}
  error={list.error instanceof Error ? list.error.message : undefined}
  {onCreate}
  {onJoin}
  onSelect={select}
  {onAddEquipment}
  {onRemoveEquipment}
  {onAddPantry}
  {onRemovePantry}
/>
