<script lang="ts">
  import Button from "./Button.svelte";
  import RowLink from "./RowLink.svelte";
  import Skeleton from "./Skeleton.svelte";
  import Panel from "./Panel.svelte";
  import type { KitchenSummary, KitchensStatus } from "$lib/types";

  /**
   * `kitchens` (#72): the kitchens you're in. That is the whole page — opening one is
   * a navigation and so is making one, so this never has to know what a kitchen
   * contains or hold a half-filled form.
   *
   * There is no empty state: everyone has a primary kitchen, made for them and named
   * after them, so a list with nothing in it is not a thing that happens. The one at
   * the top is that kitchen — the one the app works in until you open another.
   *
   * One list, because there is one kind of kitchen. A kitchen you were invited into is
   * as much yours as the one you made: everyone in it is an owner of it, and being a
   * guest is something you are at a *meal*, not in a room.
   *
   * `actionError` is what came back from redeeming an invite link, which lands on this
   * page rather than a page of its own.
   */
  interface Props {
    status: KitchensStatus;
    kitchens?: KitchenSummary[];
    error?: string;
    actionError?: string;
  }

  let { status, kitchens = [], error, actionError }: Props = $props();

</script>

<div class="pt-48 pb-16">
  <Panel>
    <p class="font-display flex items-center gap-2 text-stone-600">
      <span class="size-2.5 rounded-full bg-cocoa-500" aria-hidden="true"></span>
      Kitchens
    </p>

    {#if status === "error"}
      <p class="mt-4 text-sm text-stone-600">
        {error ?? "Couldn't load your kitchens."}
      </p>
    {:else if status === "pending"}
      <div class="mt-4"><Skeleton /></div>
    {:else}
      {#if actionError}
        <p role="alert" class="mt-3 text-sm text-paprika-500">{actionError}</p>
      {/if}

      {#snippet picker(items: KitchenSummary[])}
        {#if items.length}
          <ul class="mt-5 flex flex-col gap-2">
            {#each items as k (k.id)}
              <li>
                <RowLink href="/kitchens/{k.id}">
          <span class="flex items-baseline gap-2">
                    {k.name}
                    {#if k.is_primary}
                      <span class="text-xs text-stone-500">yours by default</span>
                    {/if}
                  </span>
        </RowLink>
              </li>
            {/each}
          </ul>
        {/if}
      {/snippet}
      {@render picker(kitchens)}

      <div class="mt-6">
        <Button href="/kitchens/new" dot="cocoa">New kitchen</Button>
      </div>
    {/if}
  </Panel>
</div>
