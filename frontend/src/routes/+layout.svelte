<script lang="ts">
  import "../app.css";
  import favicon from "$lib/assets/favicon.svg";
  import { QueryClient, QueryClientProvider } from "@tanstack/svelte-query";

  let { children } = $props();
  const queryClient = new QueryClient();
</script>

<svelte:head>
  <link rel="icon" href={favicon} />
  <title>recipes</title>
</svelte:head>

<QueryClientProvider client={queryClient}>
  {@render children()}
</QueryClientProvider>
