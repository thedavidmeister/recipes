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

use crate::models::{Ingredient, Recipe, RecipeSummary};

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
            out.push(Ingredient { name, measure });
        }
        out
    }

    fn into_summary(self) -> RecipeSummary {
        RecipeSummary {
            id: self.id,
            source: SOURCE.to_string(),
            title: self.title,
            image: self.thumb,
            category: self.category,
            area: self.area,
        }
    }

    fn into_recipe(self) -> Recipe {
        let ingredients = self.ingredients();
        let tags = self.tags.as_deref().map(split_tags).unwrap_or_default();
        Recipe {
            id: self.id,
            source: SOURCE.to_string(),
            title: self.title,
            image: self.thumb,
            category: self.category,
            area: self.area,
            tags,
            ingredients,
            instructions: self.instructions.unwrap_or_default(),
            source_url: self.source_url.filter(|s| !s.is_empty()),
            video_url: self.youtube.filter(|s| !s.is_empty()),
        }
    }
}

fn split_tags(tags: &str) -> Vec<String> {
    tags.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Normalize a TheMealDB `search.php` / `filter.php` response into summaries.
pub fn normalize_meals(json: &str) -> Vec<RecipeSummary> {
    serde_json::from_str::<MealsResponse>(json)
        .ok()
        .and_then(|r| r.meals)
        .unwrap_or_default()
        .into_iter()
        .map(Meal::into_summary)
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

    #[test]
    fn empty_response_normalizes_to_empty() {
        assert!(normalize_meals(r#"{"meals":null}"#).is_empty());
        assert!(normalize_meal(r#"{"meals":null}"#).is_none());
    }
}
