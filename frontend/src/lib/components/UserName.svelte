<script lang="ts">
  import { userColour } from "$lib/colour";

  /**
   * A person, as they appear in a list (#145): their colour, then their name.
   *
   * The two ship as one component on purpose. The colour is what makes a familiar
   * face findable in a list at a glance, but the ring is small enough that colours
   * repeat, and not everyone separates two of them — so the name is always there,
   * and the dot is decorative to a screen reader. Rendering both from here is what
   * keeps that true on every surface a person shows up on, this one and the next.
   *
   * Identity is the Telegram numeric id; the username is a display convenience and
   * may be absent (a Telegram account need not have one), so the id is the fallback
   * label as well as the colour's input.
   */
  interface Props {
    /** The person — a kitchen member or a plan's voter; both are this shape. */
    user: { telegram_user_id: string; username: string | null };
  }

  let { user }: Props = $props();
</script>

<span class="font-display flex items-center gap-2 text-stone-900">
  <span
    class="size-2.5 shrink-0 rounded-full {userColour(user.telegram_user_id)}"
    aria-hidden="true"
  ></span>
  {user.username ? `@${user.username}` : user.telegram_user_id}
</span>
