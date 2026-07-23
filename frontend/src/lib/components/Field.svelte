<script lang="ts">
  /**
   * A labelled thing to type in.
   *
   * The label is not optional and not a placeholder: a placeholder disappears exactly
   * when you need it, which is while you are typing and have forgotten what the box
   * was for.
   */
  interface Props {
    id: string;
    label: string;
    value: string;
    placeholder?: string;
  }

  let { id, label, value = $bindable(), placeholder }: Props = $props();
</script>

<label class="text-xs text-stone-500" for={id}>{label}</label>
<input
  {id}
  {placeholder}
  bind:value
  class="rounded-card border border-stone-200 bg-cream-100 px-4 py-3 text-stone-900"
/>
