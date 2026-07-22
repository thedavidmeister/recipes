<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { goto } from "$app/navigation";
  import { listKitchens, resolveKitchen } from "$lib/kitchens";

  /**
   * Where you land.
   *
   * There is no home page: you arrive in your kitchen, because that is where a meal
   * starts. Which kitchen takes a request to answer — your primary unless you have
   * switched to another — so this is a route rather than a redirect, and it sits
   * inside the auth gate because the answer is per-person.
   *
   * Everyone has a primary, so this cannot dead-end on an empty list.
   */
  const list = createQuery(() => ({
    queryKey: ["kitchens"],
    queryFn: listKitchens,
    retry: false,
  }));

  $effect(() => {
    const kitchen = list.data && resolveKitchen(list.data);
    if (kitchen) void goto(`/kitchens/${kitchen.id}`, { replaceState: true });
  });
</script>

<div class="pt-6">
  <div class="rounded-card border border-stone-200 bg-cream-100 p-8 text-center">
    <p class="font-display flex items-center justify-center gap-2 text-stone-900">
      <span class="bg-cocoa-500 size-2.5 rounded-full" aria-hidden="true"></span>
      {list.isError ? "Couldn't open your kitchen." : "Opening your kitchen…"}
    </p>
  </div>
</div>
