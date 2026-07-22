<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { getKitchen, stashCurrentKitchen } from "$lib/kitchens";
  import type { KitchensStatus } from "$lib/types";
  import Kitchen from "$lib/components/Kitchen.svelte";

  /** One kitchen (#72). The id comes from the route, so there is no selection state. */
  const id = $derived(page.params.id ?? "");

  const detail = createQuery(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
    retry: false,
  }));

  const status = $derived<KitchensStatus>(
    detail.isError ? "error" : detail.isPending ? "pending" : "ready",
  );

  const inviteLink = $derived(
    detail.data && typeof window !== "undefined"
      ? `${window.location.origin}/kitchens?join=${detail.data.invite_token}`
      : undefined,
  );

  // Remember the kitchen you last opened — the meal flow will scope itself to it.
  $effect(() => {
    if (detail.data) stashCurrentKitchen(detail.data.id);
  });
</script>

<Kitchen
  {status}
  {inviteLink}
  kitchen={detail.data}
  error={detail.error instanceof Error ? detail.error.message : undefined}
/>
