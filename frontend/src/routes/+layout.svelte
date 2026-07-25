<script lang="ts">
  import "../app.css";
  import favicon from "$lib/assets/favicon.svg";
  import { QueryClient, QueryClientProvider } from "@tanstack/svelte-query";
  import { retryTransient } from "$lib/client";

  let { children } = $props();
  // One retry policy for every query in the app: patient with a server that has not
  // woken up, and unargumentative with one that has answered. See `retryTransient`.
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: retryTransient } },
  });
</script>

<svelte:head>
  <link rel="icon" href={favicon} />
  <title>recipes</title>
</svelte:head>

<QueryClientProvider client={queryClient}>
  {@render children()}
</QueryClientProvider>
