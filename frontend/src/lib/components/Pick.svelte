<script lang="ts">
  import Alert from "./Alert.svelte";
  import Notice from "./Notice.svelte";
  import Panel from "./Panel.svelte";
  import UserName from "./UserName.svelte";
  import { userTint } from "$lib/colour";
  import { waitingOnOthers } from "$lib/deal";
  import { calorieHint, formatCalories } from "$lib/nutrition";
  import { formatEstimate } from "$lib/steps";
  import type { Voter } from "$lib/pick";
  import type { PickStatus, RecipeCard } from "$lib/types";

  /**
   * The pick swipe view (#20) — an endless, shared swipe for **consensus**.
   *
   * A pick keeps serving cards until everyone agrees on **one** recipe; the instant
   * that happens the page whisks everyone straight to `buy` (its ingredients), so
   * this view is purely the swipe. Presentational only: the page owns the socket,
   * the deck (which refills endlessly), and the cross-pollination. Every state is a
   * Storybook story.
   *
   * A card mostly gets here *because* somebody voted it — that is what
   * cross-pollination is — so the people who already said yes are named under it,
   * each in their own colour (#131/#145). It turns a solitary sort into the
   * conversation it actually is: you are not rating recipes, you are answering Mel.
   *
   * **Watching is one of its states (#180).** Someone who opened the link after the
   * swiping began has no seat, so their vote is refused where it is written — and a
   * vote is a socket frame the server never answers, so a swipe from them lands
   * nowhere and says nothing. There is no error to render after the fact, which is
   * why the state is on screen before it: the card and the tally stay (watching is
   * seeing what is being decided), and the two buttons give way to one line naming
   * whose decision this is.
   */
  interface Props {
    status: PickStatus;
    /** The card at the top of this client's deck, if any. */
    card?: RecipeCard;
    /** How many a recipe has to win over — the plan's roster, not who has swiped so
     * far (#181). One when you are the only one in the plan. */
    participants?: number;
    /** Who has already said yes to this card. */
    yesVoters?: Voter[];
    /**
     * Watching rather than deciding (#180) — this plan started without you.
     *
     * The vote controls go; nothing else does. It is not a lesser view of the pick,
     * it is the same view with the half you cannot use taken off it.
     */
    watching?: boolean;
    /**
     * Who is deciding — the plan's roster, for the watching line to name.
     *
     * Only read while `watching`: a decider is one of these people and does not need
     * telling. Empty falls back to the count, which is a lobby not read yet rather
     * than a plan with nobody in it — a started plan always holds at least its host.
     */
    roster?: Voter[];
    /**
     * Whether this person has answered every recipe the plan can currently deal them
     * (#202), so an empty deck is **finished** rather than loading.
     *
     * A prop of its own rather than another [`PickStatus`], because it is not a phase of
     * the connection like the others — it is a fact about the deal, and it is true or
     * false independently of every one of them. As a status it would have had to lose a
     * race with `reconnecting`, which would put "Finding more recipes…" back in front of
     * exactly the person this state exists for.
     *
     * **`watching` outranks it.** Both can arrive true — the walk deals a watcher a deck
     * like anyone else and an empty deal sets this — but the notice is a decider's: a
     * watcher swipes nothing, so "you've seen every recipe that fits this plan" is a
     * claim about answers they were never able to give, and the others it says are still
     * deciding are a roster they are not on. So a watcher's empty deck stays the watching
     * one (#180), whose footer already says whose decision this is.
     */
    finished?: boolean;
    error?: string;
    /** The shareable link that invites others into this pick. */
    shareUrl?: string;
    copied?: boolean;
    onVote?: (yes: boolean) => void;
    onShare?: () => void;
  }

  let {
    status,
    card,
    participants = 1,
    yesVoters = [],
    watching = false,
    roster = [],
    finished = false,
    error,
    shareUrl,
    copied = false,
    onVote,
    onShare,
  }: Props = $props();

  const meta = $derived(
    card ? [card.category, card.area].filter(Boolean).join(" · ") : "",
  );

  /**
   * "How long does this take" as a badge, so it factors into the yes/no at a glance
   * instead of surfacing after the pick (#84). Null for a recipe nobody has timed:
   * that is unknown, not instant, so the badge simply is not there.
   *
   * The card's own `fully_timed` picks the mark (#158): `~23 min` when every step of
   * it carries a duration, `23 min+` when one does not and the total can therefore
   * only be too low. Per card, not per deploy — the corpus is re-read a recipe at a
   * time, so two cards in one deck can honestly wear different marks.
   */
  const estimate = $derived(
    card ? formatEstimate(card.total_seconds, card.fully_timed) : null,
  );

  /**
   * What the badge's mark means, on hover. It has to follow `fully_timed` for the
   * same reason the mark does: a `~19 min` card carries no untimed steps, so telling
   * its reader the number is only a floor is simply wrong about that card. One
   * tooltip for both marks said the `+` sentence over every `~`.
   */
  const estimateHint = $derived(
    card?.fully_timed
      ? "Roughly how long it takes, start to finish."
      : "At least this long — some steps here have no time on them, so the cooking runs longer.",
  );

  /**
   * "What does this cost me" as a second badge, for the same reason the first one is
   * here: #162 opens on wanting it *"so a pick can be made with that in view"*, and
   * this is the only screen where that pick is being made. Everything downstream —
   * `buy`, `cook` — is after the decision, where the number can no longer change it.
   *
   * **Per serving**, which is the only interpretable form: the corpus stores the
   * whole-recipe total, and dividing it by the servings reading is a job `nutrition.ts`
   * does once for every surface rather than a stored third column free to drift from
   * the two it came from. Null — an unread recipe, or one with no servings to divide
   * by — is *nothing on the card*, never a zero and never the ambiguous total instead.
   *
   * The card's own `kcal_complete` picks the mark, exactly as `fully_timed` does above:
   * `~410 kcal a serving` when every line that stated a number was weighed, and
   * `410 kcal+ a serving` when one could not be, so the dish can only cost more.
   */
  const calories = $derived(
    card ? formatCalories(card.kcal, card.servings, card.kcal_complete) : null,
  );
</script>

<div class="pt-32 pb-16">
  <Panel>
    <header class="mb-6 flex items-center justify-between gap-4">
      <p class="font-display flex items-center gap-2 text-stone-600">
        <span class="bg-pesto-500 size-2.5 rounded-full" aria-hidden="true"
        ></span>
        Pick
      </p>
      {#if shareUrl}
        <button
          onclick={() => onShare?.()}
          class="rounded-pill font-display bg-cream-50 hover:border-pesto-500 inline-flex items-center gap-2 border border-stone-200 px-4 py-2 text-sm font-medium text-stone-900 transition-colors"
        >
          {copied ? "Link copied" : "Invite"}
        </button>
      {/if}
    </header>

    {#if status === "error"}
      <Alert>
        <p class="font-display text-stone-900">Lost the connection.</p>
        <p class="mt-1 text-sm text-stone-600">
          {error ?? "Reload the page to rejoin the others."}
        </p>
      </Alert>
    {:else if status === "connecting"}
      <Notice>
        <p class="font-display text-stone-900">Starting a pick…</p>
        <p class="mt-1 text-sm text-stone-600">
          Catching up on the votes so far.
        </p>
      </Notice>
    {:else}
      {#if status === "reconnecting"}
        <p
          class="rounded-pill bg-honey-100 mb-3 inline-flex items-center gap-2 px-3 py-1 text-sm text-stone-600"
        >
          <span class="bg-honey-500 size-2 rounded-full" aria-hidden="true"
          ></span>
          Reconnecting…
        </p>
      {/if}

      {#if !card && finished && !watching}
        <!-- An empty deck, for the other reason: everything this plan can deal you has
             been answered (see the `finished` prop). Waiting on the others is a real
             state and it says so, rather than hunting for a card that is not coming.
             Not an error and nothing to do, so it wears the same quiet Notice the other
             empty states wear, with the roster it is waiting on named under it the way
             the footer already names it. A decider's state only — a watcher has answered
             nothing and is on no roster to be waited on, so `watching` takes it off. -->
        <Notice>
          <p class="font-display text-stone-900">Nothing left to swipe.</p>
          <p class="mt-1 text-sm text-stone-600">
            You've seen every recipe that fits this plan. {waitingOnOthers(
              participants,
            )}
          </p>
        </Notice>
      {:else if !card}
        <Notice>
          <p class="font-display text-stone-900">Finding more recipes…</p>
          <p class="mt-1 text-sm text-stone-600">
            A pick keeps going until everyone agrees — the next card is on its
            way.
          </p>
        </Notice>
      {:else}
        <article
          class="rounded-card bg-cream-100 overflow-hidden border border-stone-200"
        >
          {#if card.image}
            <img
              src={card.image}
              alt={card.title}
              class="rounded-card aspect-video w-full object-cover"
              loading="lazy"
            />
          {/if}
          <div class="p-5">
            <h2 class="font-display text-xl font-medium text-stone-900">
              {card.title}
            </h2>
            <!-- Supporting information, not the headline: the meal is still the
               title and the photo. Both estimates sit beside the category/area on
               the same quiet line, in the order a decision is made in — can I be
               bothered, then can I afford it — and each is absent entirely when
               unknown. Two badges, one line, no promotion of either: what the card
               says loudly is still the dish (#84). -->
            {#if meta || estimate || calories}
              <p
                class="mt-1 flex flex-wrap items-center gap-2 text-sm text-stone-500"
              >
                {#if meta}<span>{meta}</span>{/if}
                {#if estimate}
                  <span
                    class="rounded-pill bg-cream-200 px-2 py-0.5 text-xs text-stone-600"
                    title={estimateHint}>{estimate}</span
                  >
                {/if}
                {#if calories}
                  <span
                    class="rounded-pill bg-cream-200 px-2 py-0.5 text-xs text-stone-600"
                    title={calorieHint(card.kcal_complete)}>{calories}</span
                  >
                {/if}
              </p>
            {/if}
          </div>
        </article>

        {#if yesVoters.length}
          <!-- Whose yes this already is. The tint carries the person and the name
             carries the meaning: the colour is never the only signal, which is what
             lets all six slots be used, pale ones included. -->
          <p class="mt-4 text-xs text-stone-500">Already a yes for</p>
          <ul class="mt-2 flex flex-wrap gap-2">
            {#each yesVoters as v (v.telegram_user_id)}
              <li
                class="rounded-pill px-3 py-1 text-sm {userTint(
                  v.telegram_user_id,
                )}"
              >
                <UserName user={v} />
              </li>
            {/each}
          </ul>
        {/if}

        {#if !watching}
          <!-- Absent, not disabled, for a watcher. A greyed-out Yes still offers
             itself and still explains nothing; the honest thing is that there is
             no vote here to cast, and the footer says whose it is. -->
          <div class="mt-5 flex items-center justify-center gap-4">
            <button
              onclick={() => onVote?.(false)}
              class="rounded-pill font-display bg-cream-50 border border-stone-200 px-8 py-3 font-medium text-stone-600 transition-colors hover:border-stone-400"
            >
              Pass
            </button>
            <button
              onclick={() => onVote?.(true)}
              class="rounded-pill font-display bg-pesto-500 text-cream-50 hover:bg-pesto-500/90 px-8 py-3 font-medium transition-colors"
            >
              Yes
            </button>
          </div>
        {/if}
      {/if}

      <footer class="mt-6 border-t border-stone-200 pt-4">
        {#if watching}
          <!-- The line that says whose decision this is. It takes the slot the count
             already had, because it answers the same question with the part a
             watcher is missing — not how many, but who, and why there is nothing to
             press. It sits here rather than where the buttons were so that it is on
             screen in *every* watching state, including the ones with no card yet.
             Each name wears its own colour, the app's one device for whose a thing
             is, and the sentence around them wears the resting voice every other
             quiet line in this panel wears. -->
          <p class="text-sm text-stone-500">
            {#if roster.length}
              <!-- Each name keeps its own separator so the punctuation cannot
                 drift away from the name it belongs to, and the spacing is in the
                 string rather than between the tags: Svelte trims the whitespace
                 at an each block's edges, so a space left to the markup vanishes
                 and the next person's colour lands against the comma. -->
              {#each roster as v, i (v.telegram_user_id)}
                <span
                  ><UserName user={v} inline />{i < roster.length - 2
                    ? ", "
                    : i === roster.length - 2
                      ? " and "
                      : ""}</span
                >
              {/each}
              {roster.length === 1 ? "is" : "are"} deciding.
            {:else}
              {participants} deciding.
            {/if}
            You're watching — this plan started without you.
          </p>
        {:else}
          <p class="text-sm text-stone-500">
            {participants} deciding · swipe to find something everyone likes
          </p>
        {/if}
      </footer>
    {/if}
  </Panel>
</div>
