//! The load-bearing claim, as a test: the distinctiveness/tabu strategies give
//! more variety than the naïve uniform walk on a corpus with hub structure.
//!
//! If this ever fails, the premise of the whole crate is wrong — a new strategy
//! is not beating the control — so it is worth failing loudly. Metrics are
//! averaged over many seeds so the assertion is about the strategy, not a lucky
//! walk.

use recipe_walk::eval::{clustered_corpus, variety, VarietyReport};
use recipe_walk::{
    seeded_walk, DistinctivenessWeighted, NextStep, RecipeId, TabuWeighted, UniformWalk,
};

/// Mean variety of `strategy` over 30 seeded 300-step walks of the corpus.
fn mean_variety(
    graph: &recipe_walk::FixtureGraph,
    categories: &[u32],
    strategy: &dyn NextStep,
) -> VarietyReport {
    let (steps, window) = (300, 8);
    let mut agg = VarietyReport {
        steps,
        distinct_recipe_ratio: 0.0,
        immediate_repeat_rate: 0.0,
        mean_distinct_categories: 0.0,
        category_entropy: 0.0,
    };
    let runs = 30u64;
    for seed in 0..runs {
        let walk: Vec<_> = seeded_walk(graph, strategy, seed, RecipeId(0), 5)
            .take(steps)
            .collect();
        let r = variety(&walk, categories, window);
        agg.distinct_recipe_ratio += r.distinct_recipe_ratio;
        agg.immediate_repeat_rate += r.immediate_repeat_rate;
        agg.mean_distinct_categories += r.mean_distinct_categories;
        agg.category_entropy += r.category_entropy;
    }
    let n = runs as f64;
    agg.distinct_recipe_ratio /= n;
    agg.immediate_repeat_rate /= n;
    agg.mean_distinct_categories /= n;
    agg.category_entropy /= n;
    agg
}

#[test]
fn weighted_strategies_beat_the_naive_walk() {
    let (graph, categories) = clustered_corpus(1, 6, 30, 90, 4);

    let uniform = mean_variety(&graph, &categories, &UniformWalk);
    let distinct = mean_variety(&graph, &categories, &DistinctivenessWeighted::default());
    let tabu = mean_variety(&graph, &categories, &TabuWeighted::default());

    // Distinctiveness weighting must measurably cut same-category clustering.
    // (Observed ~0.36 → ~0.21; assert a clear margin, not the exact figure.)
    assert!(
        distinct.immediate_repeat_rate < uniform.immediate_repeat_rate - 0.05,
        "distinct repeat {:.3} not clearly below uniform {:.3}",
        distinct.immediate_repeat_rate,
        uniform.immediate_repeat_rate
    );
    assert!(
        tabu.immediate_repeat_rate < uniform.immediate_repeat_rate - 0.02,
        "tabu repeat {:.3} not below uniform {:.3}",
        tabu.immediate_repeat_rate,
        uniform.immediate_repeat_rate
    );

    // Tabu should spread widest locally — most distinct categories per window.
    assert!(
        tabu.mean_distinct_categories > uniform.mean_distinct_categories,
        "tabu cats/win {:.3} not above uniform {:.3}",
        tabu.mean_distinct_categories,
        uniform.mean_distinct_categories
    );
}
