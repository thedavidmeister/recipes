<script lang="ts">
  /**
   * What to eat.
   *
   * Not built. There used to be a search box and a category browse here; both
   * are gone (#49). Search was never the plan — the client no longer decides
   * what enters the corpus at all: a schedule pulls every source's catalog
   * server-side, and this page's job is to *wander* what is already there.
   *
   * That wander is the walk (#47, `recipe-walk`), which needs structured
   * ingredients (#11) before it can run over the live corpus. Deliberately empty
   * until then, rather than a fake list or a stray "sync" button that would put
   * ingestion back in the browser's hands.
   */
</script>

<p class="mt-8 text-stone-500">Nothing to pick yet — the corpus is still filling.</p>
