// The recipe-core normalization logic, compiled to WASM (crates/recipe-wasm).
// The web-target module needs one-time init before its functions are called.
import init, {
  parseSchemaOrg,
  normalizeThemealdbSearch,
  normalizeThemealdbMeal,
  normalizeThemealdbCategories,
} from "recipe-wasm";

let ready: Promise<unknown> | null = null;

/** Initialize the WASM module once (idempotent). Call before using the exports. */
export function ensureWasm(): Promise<unknown> {
  return (ready ??= init());
}

export {
  parseSchemaOrg,
  normalizeThemealdbSearch,
  normalizeThemealdbMeal,
  normalizeThemealdbCategories,
};
