/**
 * A colour per person (#145).
 *
 * Everyone reads as themselves at a glance: the same colour beside their name in
 * the kitchen, in the lobby, and anywhere attribution lands later — without a
 * column, an endpoint, or a preference anyone has to set. It is an *assignment
 * from the palette*, never a generated colour: the design system is fenced, so a
 * colour that is not a token in `app.css` is not a colour we have.
 *
 * The ring is the four food accents plus the two flavour tones that cover the
 * hues they don't — teal and purple. Those two are only ever ingredient chips
 * elsewhere, and an ingredient chip never sits beside a person's name, so the two
 * readings never meet on a surface. Six slots, so a kitchen of the size this app
 * is for mostly gets six different people.
 *
 * Collisions are fine, and expected: six slots against however many people share
 * a kitchen will repeat. That is the deal this shape accepts — the colour rides
 * *beside* the name as a reading aid, and is never the identity itself.
 *
 * Each slot carries three classes, because a person shows up in three weights and
 * a colour has to survive all of them (#131):
 *
 * - `dot` — the solid `-500` mark beside a name. The saturated end of the hue.
 * - `tint` — the `-100` wash a thing they have claimed is filled with: a ticked
 *   shopping line, a pill the host chose. A tint rather than the solid, because a
 *   `-500` fill would have to carry text, and no single text colour is legible on
 *   all six (`honey-500` is nearly as light as the paper; `berry-500` is nearly as
 *   dark as the ink). On the `-100`s the ink never changes and stays past 12:1.
 * - `accent` — the same `-500` as `accent-color` on a native checkbox, so the box
 *   a person ticked is filled in *their* colour. The browser draws the tick itself
 *   and picks its own contrasting mark against the accent, which is exactly the
 *   part we cannot spell out per slot.
 *
 * **The colour is never the only signal.** It rides on top of something achromatic
 * every time — a checked box and a struck-through line, a pressed pill and its
 * dot, a name spelled out next to it. That is what lets all six slots be used
 * unconditionally: `honey-500` on cream measures 2.0:1 and `sea-500` 3.8:1, below
 * the 3:1 floor a control's *boundary* would need, so nothing here is ever asked
 * to be that boundary.
 *
 * The classes are spelled out per slot, not assembled from the token name, so
 * Tailwind can see them in the source and actually generate them — the same trap
 * `Nav.svelte`'s stops document.
 */
export const USER_COLOURS = [
  {
    token: "pesto",
    dot: "bg-pesto-500",
    tint: "bg-pesto-100",
    accent: "accent-pesto-500",
    edge: "border-pesto-500",
  },
  {
    token: "plum",
    dot: "bg-plum-500",
    tint: "bg-plum-100",
    accent: "accent-plum-500",
    edge: "border-plum-500",
  },
  {
    token: "paprika",
    dot: "bg-paprika-500",
    tint: "bg-paprika-100",
    accent: "accent-paprika-500",
    edge: "border-paprika-500",
  },
  {
    token: "honey",
    dot: "bg-honey-500",
    tint: "bg-honey-100",
    accent: "accent-honey-500",
    edge: "border-honey-500",
  },
  {
    token: "sea",
    dot: "bg-sea-500",
    tint: "bg-sea-100",
    accent: "accent-sea-500",
    edge: "border-sea-500",
  },
  {
    token: "berry",
    dot: "bg-berry-500",
    tint: "bg-berry-100",
    accent: "accent-berry-500",
    edge: "border-berry-500",
  },
] as const;

/**
 * Which of the six slots a Telegram numeric id lands on.
 *
 * The id is what identity *is* here — usernames are mutable and reassignable, so
 * a colour keyed on the handle would move when someone renamed themselves. Same
 * id, same slot, in every session and on every surface, by construction rather
 * than by anyone remembering to agree.
 *
 * The id arrives as a decimal string, so the remainder is folded a digit at a
 * time: an id wider than a double holds exactly is mapped by its true value, not
 * a rounded one. Anything that is not a digit is skipped rather than thrown over
 * — the colour is a reading aid, and failing to render a person because of one
 * would be the wrong trade.
 */
function slotFor(telegramUserId: string): (typeof USER_COLOURS)[number] {
  let slot = 0;
  for (const ch of telegramUserId) {
    const digit = ch.charCodeAt(0) - 48;
    if (digit < 0 || digit > 9) continue;
    slot = (slot * 10 + digit) % USER_COLOURS.length;
  }
  return USER_COLOURS[slot];
}

/** The solid `-500` dot class for a user — their mark beside their name. */
export function userColour(telegramUserId: string): string {
  return slotFor(telegramUserId).dot;
}

/**
 * The `-100` fill class for a user — the wash on something they have claimed.
 *
 * Attribution as a *surface* rather than a mark: a ticked shopping line or a pill
 * the host chose is filled with it, so "whose is this" is answered by the shape
 * itself instead of by a badge beside it (#131).
 */
export function userTint(telegramUserId: string): string {
  return slotFor(telegramUserId).tint;
}

/**
 * The `accent-color` class for a user — a native checkbox filled in their colour.
 *
 * The one place the solid `-500` carries something drawn on top of it, and the one
 * place we do not choose what: the browser draws the tick and picks a mark that
 * contrasts with the accent, which is what makes this safe across a hue as light
 * as honey and one as dark as berry.
 */
export function userAccent(telegramUserId: string): string {
  return slotFor(telegramUserId).accent;
}

/**
 * The border class for a user — their own edge, for a control already filled with
 * their tint.
 *
 * A cocoa outline round a fill in somebody else's hue reads as two decisions
 * arguing. The edge belongs to whoever the fill belongs to. It is reinforcement,
 * not the boundary that carries the meaning: the control is identified by its
 * `stone-900` label and its dot, which is what lets the two palest slots wear
 * their own edge like the other four rather than borrowing one.
 */
export function userEdge(telegramUserId: string): string {
  return slotFor(telegramUserId).edge;
}
