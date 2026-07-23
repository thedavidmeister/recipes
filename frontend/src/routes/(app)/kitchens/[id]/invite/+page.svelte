<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { getKitchen, mintInvite } from "$lib/kitchens";
  import KitchenInvite from "$lib/components/KitchenInvite.svelte";

  /**
   * Inviting someone into a kitchen (#72).
   *
   * The invite is minted by opening this page, not stored on the kitchen. It lasts two
   * hours, which is why it can be minted freely: a link nobody uses simply dies.
   *
   * `staleTime: 0` and no retry on purpose — every visit should hand you a fresh one
   * rather than a cached link that may already have expired while the page sat open.
   */
  const id = $derived(page.params.id ?? "");

  const detail = createQuery(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
    retry: false,
  }));

  const invite = createQuery(() => ({
    queryKey: ["kitchen-invite", id],
    queryFn: () => mintInvite(id),
    retry: false,
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

  const error = $derived(
    invite.error instanceof Error
      ? invite.error.message
      : detail.error instanceof Error
        ? detail.error.message
        : undefined,
  );
</script>

<KitchenInvite
  status={invite.isError || detail.isError
    ? "error"
    : invite.isPending || detail.isPending
      ? "pending"
      : "ready"}
  kitchen={detail.data?.name}
  {link}
  {remaining}
  {error}
  onRenew={() => void invite.refetch()}
/>
