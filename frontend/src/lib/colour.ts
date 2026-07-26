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
 * The classes are spelled out per slot, not assembled from the token name, so
 * Tailwind can see them in the source and actually generate them — the same trap
 * `Nav.svelte`'s stops document.
 */
export const USER_COLOURS = [
  { token: "pesto", dot: "bg-pesto-500" },
  { token: "plum", dot: "bg-plum-500" },
  { token: "paprika", dot: "bg-paprika-500" },
  { token: "honey", dot: "bg-honey-500" },
  { token: "sea", dot: "bg-sea-500" },
  { token: "berry", dot: "bg-berry-500" },
] as const;

/**
 * The dot class for a user, from their Telegram numeric id.
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
export function userColour(telegramUserId: string): string {
  let slot = 0;
  for (const ch of telegramUserId) {
    const digit = ch.charCodeAt(0) - 48;
    if (digit < 0 || digit > 9) continue;
    slot = (slot * 10 + digit) % USER_COLOURS.length;
  }
  return USER_COLOURS[slot].dot;
}
