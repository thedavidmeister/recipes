import type { Amount, Quantity } from "./types";

/**
 * Present a structured measure (#11) for reading. The enrich worker produces the
 * `Amount`; this only *formats* it for display. Parsing (raw text → structure) and
 * arithmetic (scale/convert) live in `recipe-core`, never here — this turns an
 * already-structured value into text, which is a view concern.
 */

/** Common vulgar fractions, so `0.25 cup` reads as `¼ cup` the way a recipe writes it. */
const FRACTIONS: [number, string][] = [
  [1 / 4, "¼"],
  [1 / 3, "⅓"],
  [1 / 2, "½"],
  [2 / 3, "⅔"],
  [3 / 4, "¾"],
];

/** A number the way a recipe writes it — whole, or a whole plus a common fraction. */
function formatNumber(n: number): string {
  const whole = Math.floor(n);
  const frac = n - whole;
  for (const [value, glyph] of FRACTIONS) {
    if (Math.abs(frac - value) < 0.01) {
      return whole > 0 ? `${whole}${glyph}` : glyph;
    }
  }
  // Trim float noise; a JSON integer already carries no trailing ".0".
  return String(Number(n.toFixed(2)));
}

/** A quantity: an exact value, or a range like `2–3`. */
export function formatQuantity(q: Quantity): string {
  return q.kind === "exact"
    ? formatNumber(q.value)
    : `${formatNumber(q.low)}–${formatNumber(q.high)}`;
}

/**
 * The measurement of an ingredient — quantity + unit, with any size annotation in
 * parentheses (`1 can (14 oz)`). A `qualitative` amount renders its text (`to
 * taste`). Empty string only when the line stated no amount at all. Never the raw
 * measure text — that is the enrich worker's input, not a display form.
 */
export function formatAmount(amount: Amount | null): string {
  if (!amount) return "";
  if (amount.kind === "qualitative") return amount.text;
  const parts = [formatQuantity(amount.quantity)];
  if (amount.unit) parts.push(amount.unit);
  let out = parts.join(" ");
  if (amount.size) {
    const size = [formatQuantity(amount.size.quantity)];
    if (amount.size.unit) size.push(amount.size.unit);
    out += ` (${size.join(" ")})`;
  }
  return out;
}
