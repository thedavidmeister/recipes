// The recipe-core normalization logic, compiled to WASM (crates/recipe-wasm).
// The web-target module needs one-time init before its functions are called.
//
// The surface is deliberately narrow: `normalizeDocument` is the only way to
// turn a fetched document into recipes, and it fails closed on a source no
// adapter claims. Per-source normalizers are not exposed — an ungated door
// beside the gate would just be the arbitrary-domain ingestion we don't do.
import init, { normalizeDocument, normalizeThemealdbCategories } from "recipe-wasm";

let ready: Promise<unknown> | null = null;

/** Initialize the WASM module once (idempotent). Call before using the exports. */
export function ensureWasm(): Promise<unknown> {
  return (ready ??= init());
}

export { normalizeDocument, normalizeThemealdbCategories };
