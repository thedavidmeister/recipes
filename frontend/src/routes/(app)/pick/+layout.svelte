<script lang="ts">
  import PickBackdrop from "$lib/components/PickBackdrop.svelte";

  /**
   * Every pick route is in the same room, so the backdrop lives here rather than
   * being repeated per page — the lobby and the swiping share it, which is what makes
   * starting feel like staying put rather than going somewhere else.
   */
  let { children } = $props();
</script>

<PickBackdrop />
{@render children()}
