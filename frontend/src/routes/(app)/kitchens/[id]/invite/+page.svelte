<script lang="ts">
  import { resource, together } from "$lib/resource";
  import { page } from "$app/state";
  import { getKitchen, mintInvite } from "$lib/kitchens";
  import KitchenInvite from "$lib/components/KitchenInvite.svelte";

  /**
   * Inviting someone into a kitchen (#72).
   *
   * The invite is minted by opening this page, not stored on the kitchen. It lasts two
   * hours, which is why it can be minted freely: a link nobody uses simply dies.
   *
   * `staleTime: 0` and `gcTime: 0` on purpose — every visit should hand you a fresh one
   * rather than a cached link that may already have expired while the page sat open.
   */
  const id = $derived(page.params.id ?? "");

  const detail = resource(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
  }));

  const invite = resource(() => ({
    queryKey: ["kitchen-invite", id],
    queryFn: () => mintInvite(id),
    staleTime: 0,
    gcTime: 0,
  }));

  const link = $derived(
    invite.data && typeof window !== "undefined"
      ? `${window.location.origin}/kitchens?join=${invite.data.token}`
      : undefined,
  );

  /**
   * The countdown, ticked here rather than in the component: a component that reads the
   * clock renders differently every time, which no story could pin and the visual fence
   * would flag on every run.
   */
  let remaining = $state<number | undefined>();

  $effect(() => {
    const at = invite.data?.expires_at;
    if (at === undefined) return;
    const tick = () => (remaining = at - Math.floor(Date.now() / 1000));
    tick();
    const handle = setInterval(tick, 1000);
    return () => clearInterval(handle);
  });

  // Two requests, one screen: pending until both land, and the first failure is the
  // one worth showing.
  const loaded = together(invite, detail);
</script>

<KitchenInvite
  status={loaded.status}
  kitchen={detail.data?.name}
  {link}
  {remaining}
  error={loaded.error}
  onRenew={() => void invite.query.refetch()}
/>
