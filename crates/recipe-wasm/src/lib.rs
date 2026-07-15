//! WebAssembly bindings for [`recipe_core`].
//!
//! The SvelteKit frontend imports the `wasm-pack`-built package and calls these
//! to normalize raw bytes (fetched from a source) in the browser. All logic
//! lives in `recipe-core`; this crate is only the JS boundary.
//!
//! **The boundary is deliberately narrow.** Ingestion has exactly one door —
//! [`normalize_document`] — which routes through the adapter registry and fails
//! closed on an unknown source. Per-source normalizers are *not* exported: a
//! `parseSchemaOrg(html, url)` binding would let any caller normalize any page
//! from any host, which is precisely the arbitrary-domain ingestion the corpus
//! does not do. A gate is worthless with an ungated door beside it.

use wasm_bindgen::prelude::*;

use recipe_core::{adapters, themealdb};

fn js_err(e: serde_wasm_bindgen::Error) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// Normalize a document fetched from a supported source into an array of
/// recipes. **Throws** when the URL is unparseable, or when no adapter claims
/// its host — the corpus is a cache of sources we support, not arbitrary pages,
/// so an unknown source fails closed rather than being parsed best-effort.
///
/// The host is derived from `url` inside recipe-core, never passed alongside it:
/// otherwise a caller could name a supported host for someone else's document.
#[wasm_bindgen(js_name = normalizeDocument)]
pub fn normalize_document(url: &str, body: &str) -> Result<JsValue, JsValue> {
    match adapters::normalize(url, body) {
        Ok(recipes) => serde_wasm_bindgen::to_value(&recipes).map_err(js_err),
        Err(e) => Err(JsValue::from_str(&e.to_string())),
    }
}

/// Normalize a TheMealDB `categories.php` response into an array of categories.
///
/// Categories are a browse taxonomy, not recipes — this is not an ingestion
/// path, so it does not go through the adapter gate.
#[wasm_bindgen(js_name = normalizeThemealdbCategories)]
pub fn normalize_themealdb_categories(json: &str) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&themealdb::normalize_categories(json)).map_err(js_err)
}
