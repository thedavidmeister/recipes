//! The corpus, abstracted.
//!
//! [`RecipeGraph`] is the only view the walk has of the data — a bipartite graph
//! of recipes and ingredients. Production supplies an implementation that reads
//! the normalized `recipes` view in Turso; tests and experiments use
//! [`FixtureGraph`]. Keeping this a trait is what lets a strategy be evaluated
//! over ten thousand deterministic walks offline, with no database in sight.

use crate::{IngredientId, RecipeId};

/// A recipe↔ingredient bipartite graph, queried both ways.
///
/// Implementations own the meaning of the ids (see [`RecipeId`]). They must be
/// consistent: if `r` appears in `recipes_with(i)`, then `i` must appear in
/// `ingredients_of(r)`.
pub trait RecipeGraph {
    /// The ingredients of a recipe. Unknown recipe → empty.
    fn ingredients_of(&self, recipe: RecipeId) -> &[IngredientId];

    /// The recipes containing an ingredient. Unknown ingredient → empty.
    fn recipes_with(&self, ingredient: IngredientId) -> &[RecipeId];

    /// How many recipes an ingredient appears in — its degree. The signal
    /// distinctiveness weighting reads (a hub like salt has a huge frequency, a
    /// bridge like miso a small one). The default counts `recipes_with`; a
    /// database-backed graph can override it with a precomputed tally so it need
    /// not load the whole list.
    fn frequency(&self, ingredient: IngredientId) -> u32 {
        self.recipes_with(ingredient).len() as u32
    }
}

/// An in-memory [`RecipeGraph`] for tests and offline experiments.
///
/// Built from recipes-as-ingredient-lists; the inverted index (ingredient →
/// recipes) is derived once at construction. Ids are the positions in the input:
/// recipe `r` is the `r`-th list, ingredient ids are whatever the lists contain.
pub struct FixtureGraph {
    by_recipe: Vec<Vec<IngredientId>>,
    by_ingredient: Vec<Vec<RecipeId>>,
}

impl FixtureGraph {
    /// Build from `recipes[r]` = the ingredients of recipe `r`.
    pub fn new(recipes: Vec<Vec<IngredientId>>) -> Self {
        let max_ingredient = recipes.iter().flatten().map(|i| i.0).max();
        let mut by_ingredient: Vec<Vec<RecipeId>> = match max_ingredient {
            Some(m) => vec![Vec::new(); m as usize + 1],
            None => Vec::new(),
        };
        for (r, ingredients) in recipes.iter().enumerate() {
            for &i in ingredients {
                by_ingredient[i.0 as usize].push(RecipeId(r as u32));
            }
        }
        Self {
            by_recipe: recipes,
            by_ingredient,
        }
    }

    /// The number of recipes in the graph.
    pub fn recipe_count(&self) -> usize {
        self.by_recipe.len()
    }
}

impl RecipeGraph for FixtureGraph {
    fn ingredients_of(&self, recipe: RecipeId) -> &[IngredientId] {
        self.by_recipe
            .get(recipe.0 as usize)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn recipes_with(&self, ingredient: IngredientId) -> &[RecipeId] {
        self.by_ingredient
            .get(ingredient.0 as usize)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverted_index_and_frequency() {
        // ingredient 1 is shared by recipes 0 and 1; ingredient 0 only by recipe 0.
        let g = FixtureGraph::new(vec![
            vec![IngredientId(0), IngredientId(1)],
            vec![IngredientId(1), IngredientId(2)],
        ]);
        assert_eq!(g.recipe_count(), 2);
        assert_eq!(g.recipes_with(IngredientId(1)), &[RecipeId(0), RecipeId(1)]);
        assert_eq!(g.recipes_with(IngredientId(0)), &[RecipeId(0)]);
        assert_eq!(g.frequency(IngredientId(1)), 2);
        assert_eq!(g.frequency(IngredientId(0)), 1);
    }

    #[test]
    fn unknown_ids_are_empty() {
        let g = FixtureGraph::new(vec![vec![IngredientId(0)]]);
        assert!(g.ingredients_of(RecipeId(99)).is_empty());
        assert!(g.recipes_with(IngredientId(99)).is_empty());
        assert_eq!(g.frequency(IngredientId(99)), 0);
    }
}
