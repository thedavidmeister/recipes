//! WebAssembly bindings for [`recipe_core`].
//!
//! The SvelteKit frontend imports the `wasm-pack`-built package and calls these
//! to parse/normalize raw bytes (fetched via the backend proxy) in the browser.
//! Each returns a plain JS value via `serde-wasm-bindgen`, or `null` when there
//! is nothing to return. All parsing logic lives in `recipe-core`; this crate is
//! only the JS boundary.

use wasm_bindgen::prelude::*;

use recipe_core::{adapters, schema_org, themealdb};

fn js_err(e: serde_wasm_bindgen::Error) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// Normalize a document fetched from a supported source into an array of
/// recipes. **Throws `unsupported source: <host>`** when no adapter claims the
/// host — the corpus is a cache of sources we support, not arbitrary pages, so
/// an unknown source fails closed rather than being parsed best-effort.
///
/// `host` is passed in already parsed (`new URL(u).hostname`): recipe-core does
/// not parse URLs, to keep `url`/`idna` out of the wasm bundle.
#[wasm_bindgen(js_name = normalizeDocument)]
pub fn normalize_document(host: &str, url: &str, body: &str) -> Result<JsValue, JsValue> {
    match adapters::normalize(host, url, body) {
        Ok(recipes) => serde_wasm_bindgen::to_value(&recipes).map_err(js_err),
        Err(e) => Err(JsValue::from_str(&format!("unsupported source: {}", e.host))),
    }
}

/// Whether a host is a source we ingest.
#[wasm_bindgen(js_name = isSupportedSource)]
pub fn is_supported_source(host: &str) -> bool {
    adapters::adapter_for(host).is_some()
}

/// Extract a normalized recipe from a page's HTML (its schema.org/Recipe
/// JSON-LD). Returns the recipe object, or `null` if the page has no recipe.
#[wasm_bindgen(js_name = parseSchemaOrg)]
pub fn parse_schema_org(html: &str, url: &str) -> Result<JsValue, JsValue> {
    match schema_org::parse_html(html, url) {
        Some(recipe) => serde_wasm_bindgen::to_value(&recipe).map_err(js_err),
        None => Ok(JsValue::NULL),
    }
}

/// Normalize a TheMealDB `search.php` / `filter.php` response into an array of
/// recipes (browse results come back partially populated).
#[wasm_bindgen(js_name = normalizeThemealdbSearch)]
pub fn normalize_themealdb_search(json: &str) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&themealdb::normalize_meals(json)).map_err(js_err)
}

/// Normalize a TheMealDB `lookup.php` response into one recipe, or `null`.
#[wasm_bindgen(js_name = normalizeThemealdbMeal)]
pub fn normalize_themealdb_meal(json: &str) -> Result<JsValue, JsValue> {
    match themealdb::normalize_meal(json) {
        Some(recipe) => serde_wasm_bindgen::to_value(&recipe).map_err(js_err),
        None => Ok(JsValue::NULL),
    }
}

/// Normalize a TheMealDB `categories.php` response into an array of categories.
#[wasm_bindgen(js_name = normalizeThemealdbCategories)]
pub fn normalize_themealdb_categories(json: &str) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&themealdb::normalize_categories(json)).map_err(js_err)
}
