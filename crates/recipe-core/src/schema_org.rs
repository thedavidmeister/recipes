//! Extract a normalized recipe from a page's embedded
//! [schema.org/Recipe](https://schema.org/Recipe) JSON-LD.
//!
//! Most recipe sites publish a `<script type="application/ld+json">` block with
//! structured recipe data (for Google rich results). We find a `Recipe` node —
//! including inside a `@graph` — and map it onto our normalized [`Recipe`] type.
//! Pure and synchronous, so it runs both natively and in the browser (wasm32);
//! the caller supplies the HTML (the backend proxy fetches it).

use serde_json::Value;

use url::Url;

use crate::adapters::Ingested;
use crate::models::{Ingredient, Recipe};

pub const SOURCE: &str = "url";

/// Hosts allowed to be ingested via generic schema.org parsing.
///
/// **Deliberately empty.** The corpus is a cache of sources we support, so
/// generic parsing is no longer the way in — arbitrary domains are not ingested.
/// This adapter is kept, and demoted: if generic parsing ever earns its place
/// for a specific site, allowlist that host here rather than reopening the whole
/// web (which would mean normalizing pages an attacker authored).
const ALLOWED_HOSTS: &[&str] = &[];

/// Whether a host is allowlisted for generic schema.org parsing. See
/// [`crate::adapters`].
pub fn handles(host: &str) -> bool {
    ALLOWED_HOSTS
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

/// Normalize a page into recipes, for [`crate::adapters`]. A page carries at
/// most one recipe, so its raw payload is simply the page — no slicing to do.
pub fn normalize_document(url: &Url, body: &str) -> Vec<Ingested> {
    parse_html(body, url.as_str())
        .into_iter()
        .map(|recipe| Ingested {
            recipe,
            raw: body.to_string(),
            fetched_from: url.to_string(),
        })
        .collect()
}

/// Parse `html` (fetched from `url`) and return a normalized recipe if the page
/// embeds a schema.org/Recipe.
pub fn parse_html(html: &str, url: &str) -> Option<Recipe> {
    let node = extract_recipe_ldjson(html)?;
    Some(map_recipe(&node, url))
}

/// Scan every `<script type="application/ld+json">` block for a `Recipe` node.
///
/// Uses the `tl` HTML tokenizer (pure Rust, zero-dependency, wasm-clean) — not a
/// regex. Finding a tag amid comments, quoted attributes, and raw script text is
/// a parser's job. `scraper`/html5ever would also work but bloats the wasm
/// bundle and drags in `getrandom`.
fn extract_recipe_ldjson(html: &str) -> Option<Value> {
    let dom = tl::parse(html, tl::ParserOptions::default()).ok()?;
    let parser = dom.parser();
    for handle in dom.query_selector("script")? {
        let Some(tag) = handle.get(parser).and_then(|node| node.as_tag()) else {
            continue;
        };
        let is_ld_json = tag
            .attributes()
            .get("type")
            .flatten()
            .map(|value| {
                // Accept an optional parameter, e.g. `; charset=utf-8`.
                value
                    .as_utf8_str()
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("application/ld+json")
            })
            .unwrap_or(false);
        if !is_ld_json {
            continue;
        }
        let text = tag.inner_text(parser);
        let Ok(value) = serde_json::from_str::<Value>(text.trim()) else {
            continue;
        };
        if let Some(recipe) = find_recipe_node(&value) {
            return Some(recipe.clone());
        }
    }
    None
}

/// Recursively find a node whose `@type` is (or includes) `Recipe`, descending
/// into arrays and into every object property — so a Recipe nested under
/// `@graph`, `mainEntity`, or any other property is found.
fn find_recipe_node(value: &Value) -> Option<&Value> {
    match value {
        Value::Array(items) => items.iter().find_map(find_recipe_node),
        Value::Object(map) => {
            if is_recipe_type(map.get("@type")) {
                return Some(value);
            }
            map.values().find_map(find_recipe_node)
        }
        _ => None,
    }
}

fn is_recipe_type(ty: Option<&Value>) -> bool {
    match ty {
        Some(Value::String(s)) => s.eq_ignore_ascii_case("Recipe"),
        Some(Value::Array(items)) => items
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("Recipe"))),
        _ => false,
    }
}

fn map_recipe(node: &Value, url: &str) -> Recipe {
    Recipe {
        id: url.to_string(),
        source: SOURCE.to_string(),
        title: node
            .get("name")
            .and_then(first_string)
            .unwrap_or_else(|| "Untitled recipe".to_string()),
        image: node.get("image").and_then(first_url),
        category: node.get("recipeCategory").and_then(first_string),
        area: node.get("recipeCuisine").and_then(first_string),
        tags: string_or_array(node.get("keywords")),
        ingredients: extract_ingredients(node.get("recipeIngredient")),
        instructions: extract_instructions(node.get("recipeInstructions")),
        source_url: Some(url.to_string()),
        video_url: node
            .get("video")
            .and_then(|v| v.get("contentUrl").or_else(|| v.get("embedUrl")))
            .and_then(first_string),
    }
}

/// First scalar string reachable from `value` (unwrapping arrays and common
/// wrapper objects like `{ "@value": ... }` / `{ "name": ... }`).
fn first_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => items.iter().find_map(first_string),
        Value::Object(map) => map
            .get("@value")
            .or_else(|| map.get("name"))
            .and_then(first_string),
        _ => None,
    }
}

/// First URL reachable from `value` (image can be a string, an `ImageObject`
/// with a `url`, or an array of either).
fn first_url(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => items.iter().find_map(first_url),
        Value::Object(map) => map.get("url").and_then(first_url),
        _ => None,
    }
}

fn extract_ingredients(value: Option<&Value>) -> Vec<Ingredient> {
    let lines = match value {
        Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>(),
        Some(Value::String(s)) => vec![s.as_str()],
        _ => Vec::new(),
    };
    lines
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|name| Ingredient {
            name,
            measure: None,
        })
        .collect()
}

fn extract_instructions(value: Option<&Value>) -> String {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(instruction_text)
            .collect::<Vec<_>>()
            .join("\n\n"),
        // A single string, or a single HowToStep / HowToSection object.
        Some(value) => instruction_text(value).unwrap_or_default(),
        None => String::new(),
    }
}

/// Flatten a single `recipeInstructions` element (a string, a `HowToStep`, or a
/// `HowToSection` containing `itemListElement`s) into text.
fn instruction_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("itemListElement") {
                let joined = items
                    .iter()
                    .filter_map(instruction_text)
                    .collect::<Vec<_>>()
                    .join("\n\n");
                return Some(joined).filter(|s| !s.is_empty());
            }
            map.get("text")
                .and_then(|t| t.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        }
        _ => None,
    }
}

fn string_or_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_recipe_from_graph() {
        let html = r#"
        <html><head>
        <script type="application/ld+json">
        {
          "@context": "https://schema.org",
          "@graph": [
            { "@type": "WebPage", "name": "some page" },
            {
              "@type": "Recipe",
              "name": "Pancakes",
              "image": { "@type": "ImageObject", "url": "https://example.com/p.jpg" },
              "recipeIngredient": ["1 cup flour", "2 eggs", "  "],
              "recipeInstructions": [
                { "@type": "HowToStep", "text": "Mix the batter." },
                { "@type": "HowToStep", "text": "Cook on a griddle." }
              ],
              "recipeCategory": "Breakfast",
              "recipeCuisine": "American",
              "keywords": "easy, quick"
            }
          ]
        }
        </script></head><body></body></html>"#;

        let recipe = parse_html(html, "https://example.com/pancakes").expect("a Recipe");

        assert_eq!(recipe.title, "Pancakes");
        assert_eq!(recipe.source, "url");
        assert_eq!(recipe.image.as_deref(), Some("https://example.com/p.jpg"));
        assert_eq!(recipe.ingredients.len(), 2, "blank ingredient dropped");
        assert_eq!(recipe.ingredients[0].name, "1 cup flour");
        assert_eq!(recipe.instructions, "Mix the batter.\n\nCook on a griddle.");
        assert_eq!(recipe.category.as_deref(), Some("Breakfast"));
        assert_eq!(recipe.area.as_deref(), Some("American"));
        assert_eq!(recipe.tags, vec!["easy", "quick"]);
    }

    #[test]
    fn returns_none_without_recipe() {
        let html = r#"<script type="application/ld+json">{"@type":"WebPage"}</script>"#;
        assert!(parse_html(html, "https://example.com").is_none());
    }

    #[test]
    fn handles_type_with_charset_parameter() {
        // A parameterized media type the old exact-match regex would have missed.
        let html = r#"<script type="application/ld+json; charset=utf-8">
            {"@type":"Recipe","name":"Toast","recipeIngredient":["bread"]}
        </script>"#;
        let recipe = parse_html(html, "https://example.com/toast").expect("a Recipe");
        assert_eq!(recipe.title, "Toast");
        assert_eq!(recipe.ingredients.len(), 1);
    }

    #[test]
    fn finds_recipe_nested_under_main_entity() {
        // Very common: a WebPage wrapper with the Recipe under `mainEntity`
        // (not `@graph`), which the earlier `@graph`-only traversal missed.
        let html = r#"<script type="application/ld+json">
        {
          "@type": "WebPage",
          "mainEntity": {
            "@type": "Recipe",
            "name": "Nested Stew",
            "recipeIngredient": ["water"]
          }
        }
        </script>"#;
        let recipe = parse_html(html, "https://example.com/stew").expect("a Recipe");
        assert_eq!(recipe.title, "Nested Stew");
        assert_eq!(recipe.ingredients.len(), 1);
    }

    #[test]
    fn handles_single_instruction_object() {
        // `recipeInstructions` as a lone HowToStep object (not a string/array).
        let html = r#"<script type="application/ld+json">
        {
          "@type": "Recipe",
          "name": "One Step",
          "recipeInstructions": { "@type": "HowToStep", "text": "Just do it." }
        }
        </script>"#;
        let recipe = parse_html(html, "https://example.com/x").expect("a Recipe");
        assert_eq!(recipe.instructions, "Just do it.");
    }
}
