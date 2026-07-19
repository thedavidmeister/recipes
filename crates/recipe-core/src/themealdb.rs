//! Normalize [TheMealDB](https://www.themealdb.com/api.php) JSON payloads.
//!
//! Pure functions: the caller supplies the raw JSON text (the browser fetches
//! TheMealDB directly since it sends permissive CORS; other sources come
//! through the backend proxy) and these map it onto our normalized types. Its
//! meal payloads carry up to 20 ingredient/measure pairs as flat
//! `strIngredient{n}` / `strMeasure{n}` fields, which we fold into
//! [`Ingredient`]s.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use url::Url;

use crate::adapters::Ingested;
use crate::models::{Ingredient, Recipe};

pub const SOURCE: &str = "themealdb";

#[derive(Debug, Deserialize)]
struct MealsResponse {
    meals: Option<Vec<Meal>>,
}

#[derive(Debug, Deserialize)]
struct Meal {
    #[serde(rename = "idMeal")]
    id: String,
    #[serde(rename = "strMeal")]
    title: String,
    #[serde(rename = "strCategory")]
    category: Option<String>,
    #[serde(rename = "strArea")]
    area: Option<String>,
    #[serde(rename = "strInstructions")]
    instructions: Option<String>,
    #[serde(rename = "strMealThumb")]
    thumb: Option<String>,
    #[serde(rename = "strTags")]
    tags: Option<String>,
    #[serde(rename = "strYoutube")]
    youtube: Option<String>,
    #[serde(rename = "strSource")]
    source_url: Option<String>,
    /// Captures the `strIngredient{n}` / `strMeasure{n}` pairs.
    #[serde(flatten)]
    extra: HashMap<String, Option<String>>,
}

impl Meal {
    fn ingredients(&self) -> Vec<Ingredient> {
        let mut out = Vec::new();
        for i in 1..=20 {
            let name = self
                .extra
                .get(&format!("strIngredient{i}"))
                .cloned()
                .flatten()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let Some(name) = name else { continue };
            let measure = self
                .extra
                .get(&format!("strMeasure{i}"))
                .cloned()
                .flatten()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            out.push(Ingredient {
                name,
                measure,
                structured: None,
            });
        }
        out
    }

    fn into_recipe(self) -> Recipe {
        let ingredients = self.ingredients();
        let tags = self.tags.as_deref().map(split_tags).unwrap_or_default();
        Recipe {
            id: self.id,
            source: SOURCE.to_string(),
            title: self.title,
            image: self.thumb.and_then(|s| crate::adapters::http_url(&s)),
            category: self.category,
            area: self.area,
            tags,
            ingredients,
            instructions: self.instructions.unwrap_or_default(),
            // http(s) only — strSource/strYoutube/strMealThumb are third-party data
            // we do not control. http_url also subsumes the old empty-string filter.
            source_url: self.source_url.and_then(|s| crate::adapters::http_url(&s)),
            video_url: self.youtube.and_then(|s| crate::adapters::http_url(&s)),
        }
    }
}

fn split_tags(tags: &str) -> Vec<String> {
    tags.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Whether a host is TheMealDB. See [`crate::adapters`].
pub fn handles(host: &str) -> bool {
    host == "themealdb.com" || host.ends_with(".themealdb.com")
}

/// TheMealDB's whole catalog, for a server-driven sync (#49).
///
/// The free API has no "list everything" call, but `search.php?f=<char>` returns
/// every meal whose name starts with that character — **complete** (ingredients
/// and instructions, not the header-only shape `filter.php` gives), and
/// **unpaginated**. That last part is measured, not assumed: against the live API
/// the per-character counts run 0…118 with no repeated ceiling, so a response is
/// the whole set for that character, not a first page.
///
/// So a–z **plus 0–9** enumerate the corpus flat, with no crawl. The digits are
/// not theoretical: `f=1` returns "15-minute chicken & halloumi burgers", which an
/// a–z-only catalog silently missed. Nine of the ten digits return nothing, which
/// costs one cheap request each to stay correct as names change.
pub fn catalog() -> Vec<String> {
    (b'a'..=b'z')
        .chain(b'0'..=b'9')
        .map(|start| {
            format!(
                "https://www.themealdb.com/api/json/v1/1/search.php?f={}",
                start as char
            )
        })
        .collect()
}

/// Normalize any TheMealDB document into recipes, each paired with its own raw
/// payload, for [`crate::adapters`].
///
/// No endpoint dispatch is needed: `search.php`, `filter.php` and `lookup.php`
/// all return the same `{"meals":[…]}` envelope, and a document carrying no
/// meals (`categories.php`) normalizes to nothing.
///
/// Each recipe's `raw` is **its own meal object**, re-wrapped in the `{"meals":
/// […]}` envelope — i.e. what `lookup.php` would have returned for it alone. So
/// a 25-meal search yields 25 small payloads instead of 25 copies of the
/// response, and each one re-normalizes through this very function.
///
/// The meal travels as the JSON that arrived, not re-serialized from our own
/// [`Meal`] struct: going through the struct would store *our current reading* of
/// the source and silently drop every field this version does not yet map, which
/// is exactly what deriving later needs. Round-tripping through [`Value`] keeps
/// those fields, values and nulls intact.
///
/// It is a re-encoding, not a byte copy: `serde_json` sorts object keys and keeps
/// the last of a duplicate key, so `raw` is *equivalent* to the source's meal
/// rather than identical to it. Normalization reads fields by name, so neither
/// affects what derives from it — but do not treat `raw` as bytes the source
/// signed or as something to diff against a fresh fetch.
pub fn normalize_document(url: &Url, body: &str) -> Vec<Ingested> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let Some(meals) = value.get("meals").and_then(Value::as_array) else {
        return Vec::new();
    };
    meals
        .iter()
        .filter_map(|meal| {
            let recipe = serde_json::from_value::<Meal>(meal.clone())
                .ok()?
                .into_recipe();
            let raw = json!({ "meals": [meal] }).to_string();
            Some(Ingested {
                recipe,
                raw,
                fetched_from: url.to_string(),
            })
        })
        .collect()
}

/// Normalize a TheMealDB `search.php` / `filter.php` response. `filter.php`
/// returns only header fields, so those recipes come back partially populated.
pub fn normalize_meals(json: &str) -> Vec<Recipe> {
    serde_json::from_str::<MealsResponse>(json)
        .ok()
        .and_then(|r| r.meals)
        .unwrap_or_default()
        .into_iter()
        .map(Meal::into_recipe)
        .collect()
}

/// Normalize a TheMealDB `lookup.php` response into a single full recipe.
pub fn normalize_meal(json: &str) -> Option<Recipe> {
    serde_json::from_str::<MealsResponse>(json)
        .ok()
        .and_then(|r| r.meals)
        .and_then(|meals| meals.into_iter().next())
        .map(Meal::into_recipe)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    pub thumb: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CategoriesResponse {
    categories: Vec<RawCategory>,
}

#[derive(Debug, Deserialize)]
struct RawCategory {
    #[serde(rename = "strCategory")]
    name: String,
    #[serde(rename = "strCategoryThumb")]
    thumb: Option<String>,
    #[serde(rename = "strCategoryDescription")]
    description: Option<String>,
}

/// Normalize a TheMealDB `categories.php` response.
pub fn normalize_categories(json: &str) -> Vec<Category> {
    serde_json::from_str::<CategoriesResponse>(json)
        .map(|r| {
            r.categories
                .into_iter()
                .map(|c| Category {
                    name: c.name,
                    thumb: c.thumb,
                    description: c.description,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_a_meal_with_ingredients() {
        let json = r#"{"meals":[{
            "idMeal":"52772","strMeal":"Teriyaki Chicken","strCategory":"Chicken",
            "strArea":"Japanese","strInstructions":"Cook it.","strMealThumb":"https://img/x.jpg",
            "strTags":"Meat,Casserole","strYoutube":"","strSource":"",
            "strIngredient1":"soy sauce","strMeasure1":"1/2 cup",
            "strIngredient2":"chicken","strMeasure2":" ",
            "strIngredient3":"","strMeasure3":"1 tbsp"
        }]}"#;

        let recipe = normalize_meal(json).expect("a recipe");
        assert_eq!(recipe.title, "Teriyaki Chicken");
        assert_eq!(recipe.source, "themealdb");
        assert_eq!(recipe.area.as_deref(), Some("Japanese"));
        assert_eq!(recipe.tags, vec!["Meat", "Casserole"]);
        // ingredient 3 dropped (blank name); ingredient 2 keeps name, no measure
        assert_eq!(recipe.ingredients.len(), 2);
        assert_eq!(recipe.ingredients[0].measure.as_deref(), Some("1/2 cup"));
        assert_eq!(recipe.ingredients[1].name, "chicken");
        assert_eq!(recipe.ingredients[1].measure, None);
    }

    /// Why `raw` is built from the parsed JSON and never from [`Meal`]: a field
    /// this version does not map must still reach `raw_imports`, or deriving
    /// could only ever recover what we already knew how to read. Serializing our
    /// struct would silently drop `strCreativeCommonsConfirmed` here.
    #[test]
    fn raw_keeps_fields_the_struct_does_not_map() {
        let json = r#"{"meals":[{"idMeal":"1","strMeal":"Toast","strInstructions":"Toast it.","strIngredient1":"Bread","strMeasure1":"1","strCreativeCommonsConfirmed":"Yes","dateModified":null}]}"#;
        let url = Url::parse("https://www.themealdb.com/api/json/v1/1/lookup.php?i=1").unwrap();
        let ingested = normalize_document(&url, json);
        assert_eq!(ingested.len(), 1);

        let raw: serde_json::Value = serde_json::from_str(&ingested[0].raw).unwrap();
        let meal = &raw["meals"][0];
        assert_eq!(meal["strCreativeCommonsConfirmed"], "Yes");
        assert!(meal["dateModified"].is_null());
        assert_eq!(meal["strMeal"], "Toast");
    }

    #[test]
    fn empty_response_normalizes_to_empty() {
        assert!(normalize_meals(r#"{"meals":null}"#).is_empty());
        assert!(normalize_meal(r#"{"meals":null}"#).is_none());
    }

    #[test]
    fn hostile_url_schemes_are_dropped() {
        // strMealThumb/strYoutube/strSource are third-party; a javascript:/data:/file:
        // URL in any is refused, but the meal still normalizes.
        let json = r#"{"meals":[{
            "idMeal":"1","strMeal":"X","strInstructions":"Cook.",
            "strMealThumb":"javascript:alert(1)","strYoutube":"data:text/html,x",
            "strSource":"file:///etc/passwd",
            "strIngredient1":"Bread","strMeasure1":"1"
        }]}"#;
        let recipe = normalize_meal(json).expect("a recipe");
        assert_eq!(recipe.image, None);
        assert_eq!(recipe.video_url, None);
        assert_eq!(recipe.source_url, None);
        assert_eq!(recipe.title, "X");
    }

    #[test]
    fn valid_http_urls_pass_through() {
        let json = r#"{"meals":[{
            "idMeal":"1","strMeal":"X","strInstructions":"Cook.",
            "strMealThumb":"https://img/x.jpg",
            "strYoutube":"https://www.youtube.com/watch?v=abc",
            "strSource":"http://example.com/recipe",
            "strIngredient1":"Bread","strMeasure1":"1"
        }]}"#;
        let recipe = normalize_meal(json).expect("a recipe");
        assert_eq!(recipe.image.as_deref(), Some("https://img/x.jpg"));
        assert_eq!(
            recipe.video_url.as_deref(),
            Some("https://www.youtube.com/watch?v=abc")
        );
        assert_eq!(
            recipe.source_url.as_deref(),
            Some("http://example.com/recipe")
        );
    }
}
