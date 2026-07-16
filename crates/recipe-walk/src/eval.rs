//! Offline variety metrics — how "experiment over time" becomes numbers.
//!
//! A strategy's job is variety, which you cannot eyeball from one walk. So run
//! many seeded walks over a fixed corpus and *score* them. The scoring needs to
//! know what each recipe "is" — its category (chicken, beef, dessert) — which the
//! graph deliberately does not carry, so it is supplied here as a per-recipe
//! label. Lower repeat, higher distinct-per-window and entropy = more variety.
//!
//! [`clustered_corpus`] builds a synthetic corpus with the hub structure the real
//! one has, so the metrics can be exercised (and the strategies compared) without
//! live data.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::{HashMap, HashSet};

use crate::{FixtureGraph, IngredientId, RecipeId, Step};

/// A per-recipe category label — what the walk is trying to vary across.
pub type Category = u32;

/// The variety of a single walk.
#[derive(Debug, Clone, PartialEq)]
pub struct VarietyReport {
    /// Steps scored.
    pub steps: usize,
    /// Distinct recipes visited over distinct-possible — how much ground covered.
    pub distinct_recipe_ratio: f64,
    /// Fraction of consecutive steps in the *same* category — the
    /// "chicken-then-chicken" measure. Lower is better.
    pub immediate_repeat_rate: f64,
    /// Mean number of distinct categories in a sliding window — local variety.
    /// Higher is better.
    pub mean_distinct_categories: f64,
    /// Shannon entropy (bits) of the category distribution over the whole walk —
    /// global spread. Higher is better.
    pub category_entropy: f64,
}

/// Score a walk's variety against per-recipe `category_of` (indexed by
/// `RecipeId`), using a sliding `window` for the local measure.
pub fn variety(steps: &[Step], category_of: &[Category], window: usize) -> VarietyReport {
    let category = |r: RecipeId| category_of.get(r.0 as usize).copied();
    let n = steps.len();

    let distinct: HashSet<RecipeId> = steps.iter().map(|s| s.recipe).collect();
    let distinct_recipe_ratio = if n > 0 {
        distinct.len() as f64 / n as f64
    } else {
        0.0
    };

    let mut repeats = 0usize;
    let mut adjacent = 0usize;
    for pair in steps.windows(2) {
        if let (Some(a), Some(b)) = (category(pair[0].recipe), category(pair[1].recipe)) {
            adjacent += 1;
            if a == b {
                repeats += 1;
            }
        }
    }
    let immediate_repeat_rate = if adjacent > 0 {
        repeats as f64 / adjacent as f64
    } else {
        0.0
    };

    let mut window_sum = 0.0;
    let mut windows = 0usize;
    if window > 0 && n >= window {
        for w in steps.windows(window) {
            let cats: HashSet<Category> = w.iter().filter_map(|s| category(s.recipe)).collect();
            window_sum += cats.len() as f64;
            windows += 1;
        }
    }
    let mean_distinct_categories = if windows > 0 {
        window_sum / windows as f64
    } else {
        0.0
    };

    let mut counts: HashMap<Category, u32> = HashMap::new();
    for s in steps {
        if let Some(c) = category(s.recipe) {
            *counts.entry(c).or_insert(0) += 1;
        }
    }
    let total: u32 = counts.values().sum();
    let category_entropy = if total > 0 {
        counts
            .values()
            .map(|&c| {
                let p = c as f64 / total as f64;
                -p * p.log2()
            })
            .sum()
    } else {
        0.0
    };

    VarietyReport {
        steps: n,
        distinct_recipe_ratio,
        immediate_repeat_rate,
        mean_distinct_categories,
        category_entropy,
    }
}

/// Build a synthetic corpus that reproduces the real hub structure, so strategies
/// can be compared offline. Every recipe carries:
///
/// - a ubiquitous **staple** (ingredient `0`) — in every recipe, so hopping by it
///   is a near-random teleport (variety, but no thread);
/// - its category's **anchor** (a protein-like hub confined to that category) —
///   hopping by it keeps you in the category (the cluster trap);
/// - a few distinctive **bridges** — low-frequency ingredients sprinkled across
///   categories, so hopping by one crosses categories coherently.
///
/// Returns the graph and the category label of each recipe. Deterministic in
/// `seed`.
pub fn clustered_corpus(
    seed: u64,
    categories: u32,
    per_category: u32,
    bridges: u32,
    bridge_span: u32,
) -> (FixtureGraph, Vec<Category>) {
    let staple = IngredientId(0);
    let anchor = |c: u32| IngredientId(1 + c);
    let bridge = |b: u32| IngredientId(1 + categories + b);

    let n = (categories * per_category) as usize;
    let mut recipes: Vec<Vec<IngredientId>> = Vec::with_capacity(n);
    let mut labels: Vec<Category> = Vec::with_capacity(n);
    for c in 0..categories {
        for _ in 0..per_category {
            recipes.push(vec![staple, anchor(c)]);
            labels.push(c);
        }
    }

    // Sprinkle each bridge across `bridge_span` random recipes — likely spanning
    // categories, which is what makes it a bridge.
    let mut rng = StdRng::seed_from_u64(seed);
    for b in 0..bridges {
        for _ in 0..bridge_span {
            let r = rng.gen_range(0..n);
            recipes[r].push(bridge(b));
        }
    }

    (FixtureGraph::new(recipes), labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variety_of_a_perfectly_repetitive_walk() {
        // Every step lands in category 0 → maximal repeat, zero spread.
        let cats = vec![0; 5];
        let steps: Vec<Step> = (0..5)
            .map(|r| Step {
                via: IngredientId(0),
                recipe: RecipeId(r),
            })
            .collect();
        let report = variety(&steps, &cats, 3);
        assert_eq!(report.immediate_repeat_rate, 1.0);
        assert_eq!(report.category_entropy, 0.0);
        assert_eq!(report.mean_distinct_categories, 1.0);
    }

    #[test]
    fn variety_of_a_perfectly_varied_walk() {
        // Each step a different category → no repeats, full entropy.
        let cats = vec![0, 1, 2, 3];
        let steps: Vec<Step> = (0..4)
            .map(|r| Step {
                via: IngredientId(0),
                recipe: RecipeId(r),
            })
            .collect();
        let report = variety(&steps, &cats, 2);
        assert_eq!(report.immediate_repeat_rate, 0.0);
        assert_eq!(report.category_entropy, 2.0); // log2(4)
        assert_eq!(report.mean_distinct_categories, 2.0);
    }
}
