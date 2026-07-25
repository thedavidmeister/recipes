<script lang="ts">
  import RowLink from "./RowLink.svelte";
  import Skeleton from "./Skeleton.svelte";
  import Panel from "./Panel.svelte";
  import type { KitchenDetail, KitchensStatus } from "$lib/types";

  /**
   * A kitchen's settings (#117): the occasional things you *change* about a kitchen —
   * its name, who is in it, what it owns — gathered off the kitchen page so that page
   * can be about *being* in the kitchen rather than administering it.
   *
   * A hub, not a host. Each of these is already its own page, so settings links to
   * them rather than rebuilding them. Everyone in a kitchen is an owner (#103), so
   * none of it is permission-gated — it is set aside because it is occasional, not
   * because it is privileged.
   *
   * The way back is to the kitchen, not up to a list: a kitchen is a place you are in,
   * and settings is a page you step into and come back from, so navigation stays flat.
   */
  interface Props {
    status: KitchensStatus;
    /** The kitchen id from the route, so the way back works before the detail lands. */
    id: string;
    kitchen?: KitchenDetail | null;
    error?: string;
  }

  let { status, id, kitchen, error }: Props = $props();
</script>

<div class="pt-48 pb-16">
  <Panel>
    <a href="/kitchens/{id}" class="text-sm text-stone-500 underline"
      >← Kitchen</a
    >
    <h1 class="font-display mt-3 text-2xl font-medium text-stone-900">
      Settings
    </h1>

    {#if status === "error" || (status === "ready" && !kitchen)}
      <p class="mt-4 text-sm text-stone-600">
        {error ?? "Couldn't open this kitchen."}
      </p>
    {:else if status === "pending" || !kitchen}
      <div class="mt-4"><Skeleton /></div>
    {:else}
      <p class="mt-2 text-sm text-stone-500">
        The occasional things — its name, who's in it, what it's stocked with.
      </p>

      <ul class="mt-5 flex flex-col gap-2">
        <li>
          <RowLink href="/kitchens/{kitchen.id}/name">Rename</RowLink>
        </li>
        <li>
          <RowLink href="/kitchens/{kitchen.id}/invite">Invite someone</RowLink>
        </li>
        <li>
          <RowLink
            href="/kitchens/{kitchen.id}/equipment"
            trailing={kitchen.equipment.length}
          >
            Equipment
          </RowLink>
        </li>
        <li>
          <RowLink
            href="/kitchens/{kitchen.id}/pantry"
            trailing={kitchen.pantry.length}
          >
            Pantry
          </RowLink>
        </li>
      </ul>
    {/if}
  </Panel>
</div>
