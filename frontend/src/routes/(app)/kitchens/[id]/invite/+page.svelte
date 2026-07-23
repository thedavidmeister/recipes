<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { page } from "$app/state";
  import { getKitchen } from "$lib/kitchens";
  import KitchenInvite from "$lib/components/KitchenInvite.svelte";

  /** Inviting someone into a kitchen (#72). */
  const id = $derived(page.params.id ?? "");

  const detail = createQuery(() => ({
    queryKey: ["kitchen", id],
    queryFn: () => getKitchen(id),
    retry: false,
  }));

  const link = $derived(
    detail.data && typeof window !== "undefined"
      ? `${window.location.origin}/kitchens?join=${detail.data.invite_token}`
      : undefined,
  );
</script>

<KitchenInvite
  status={detail.isError ? "error" : detail.isPending ? "pending" : "ready"}
  kitchen={detail.data?.name}
  {link}
  error={detail.error instanceof Error ? detail.error.message : undefined}
/>
