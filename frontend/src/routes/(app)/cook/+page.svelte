<script lang="ts">
  import { createQuery } from "@tanstack/svelte-query";
  import { getCookRecipe } from "$lib/cook";
  import type { CookStatus } from "$lib/types";
  import Cook from "$lib/components/Cook.svelte";

  /**
   * `cook` (#36) — the picked recipe in full, to follow while cooking.
   *
   * The step after `buy`: reads the pick's decision (the same consensus recipe) and
   * shows the whole thing with the method emphasized. The page owns the query;
   * `Cook` renders. Read client-direct from Turso (the corpus is public), so a
   * lapsed session doesn't 401 it — the layout already gates the shell.
   */
  const recipe = createQuery(() => ({
    queryKey: ["cook"],
    queryFn: () => getCookRecipe(),
    staleTime: Infinity,
    retry: false,
  }));

  const status = $derived<CookStatus>(
    recipe.isError ? "error" : recipe.isPending ? "pending" : "ready",
  );
</script>

<Cook
  {status}
  recipe={recipe.data}
  error={recipe.error instanceof Error ? recipe.error.message : undefined}
/>
