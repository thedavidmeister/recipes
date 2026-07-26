<script lang="ts">
  import Skeleton from "./Skeleton.svelte";
  import Panel from "./Panel.svelte";
  import Button from "./Button.svelte";
  import UserName from "./UserName.svelte";
  import type { KitchenDetail, KitchensStatus } from "$lib/types";

  /**
   * One kitchen (#72): what it is, who owns it, and the thing you come here to do —
   * start a meal. The kitchen's name is the first thing on the page, because this is
   * where you land: `/` resolves to your primary kitchen, so nothing was drilled into
   * to get here. The owners sit directly under the title — who is in the kitchen is
   * the first thing you want to see; the actions come after. Changing the kitchen —
   * its name, who's in it, what it owns — lives on its settings page (#117), reached
   * by a single quiet link, so this page is about being in the kitchen rather than
   * administering it.
   *
   * The list of kitchens is **not** above this page (#119). It is a page you visit on
   * the rare occasion you want a different kitchen, so "Switch kitchen" sits at the
   * bottom in the same quiet register as "Settings", never as a back arrow at the top:
   * a link there would read as a parent you came down from, and this *is* home.
   *
   * Nobody here has a role beyond owner. Everyone in a kitchen owns it — being a
   * guest is something you are at a meal, not in a room — so the members render
   * under the one label "Owners". Each carries their colour (#145), the same one
   * they wear in the lobby, so the room reads as people rather than a list.
   */
  interface Props {
    status: KitchensStatus;
    kitchen?: KitchenDetail | null;
    error?: string;
    /** Start a meal plan in this kitchen — the lobby is the next page. */
    onPlan?: () => void;
  }

  let { status, kitchen, error, onPlan }: Props = $props();
</script>

<div class="pt-48 pb-16">
  <Panel>
    {#if status === "error" || (status === "ready" && !kitchen)}
      <p class="text-sm text-stone-600">
        {error ?? "Couldn't open this kitchen."}
      </p>
    {:else if status === "pending" || !kitchen}
      <Skeleton />
    {:else}
      <h1 class="font-display text-2xl font-medium text-stone-900">
        {kitchen.name}
      </h1>

      <p class="mt-5 mb-3 text-xs text-stone-500">Owners</p>
      <ul class="flex flex-col gap-1.5">
        {#each kitchen.members as m (m.telegram_user_id)}
          <li><UserName user={m} /></li>
        {/each}
      </ul>

      <div class="mt-8">
        <Button onclick={onPlan} dot="pesto">Let's cook!</Button>
      </div>

      <div class="mt-8 flex gap-5">
        <a
          href="/kitchens/{kitchen.id}/settings"
          class="text-sm text-stone-500 underline hover:text-stone-700"
        >
          Settings
        </a>
        <a
          href="/kitchens"
          class="text-sm text-stone-500 underline hover:text-stone-700"
        >
          Switch kitchen
        </a>
      </div>
    {/if}
  </Panel>
</div>
