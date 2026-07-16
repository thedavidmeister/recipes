//! Compare the strategies' variety over a synthetic clustered corpus.
//!
//!   cargo run --example compare -p recipe-walk
//!
//! This is the experimentation harness: add a `NextStep`, drop it in the list,
//! and read off whether it spreads the walk better than the ones before it. The
//! numbers are averaged over many seeds so a lucky walk doesn't flatter a
//! strategy.

use recipe_walk::eval::{clustered_corpus, variety};
use recipe_walk::{
    seeded_walk, DistinctivenessWeighted, NextStep, RecipeId, TabuWeighted, UniformWalk,
};

fn main() {
    // 6 categories × 30 recipes, with distinctive bridges sprinkled across them.
    let (graph, categories) = clustered_corpus(1, 6, 30, 90, 4);
    let window = 8;
    let steps = 400;
    let seeds = 0..40u64;

    let strategies: [(&str, Box<dyn NextStep>); 3] = [
        ("uniform", Box::new(UniformWalk)),
        ("distinct", Box::new(DistinctivenessWeighted::default())),
        ("tabu", Box::new(TabuWeighted::default())),
    ];

    println!(
        "{:<10} {:>8} {:>10} {:>10} {:>9}",
        "strategy", "repeat", "distinct", "cats/win", "entropy"
    );
    println!("{}", "-".repeat(50));
    for (name, strategy) in &strategies {
        let (mut repeat, mut distinct, mut cats, mut entropy) = (0.0, 0.0, 0.0, 0.0);
        let mut runs = 0.0;
        for seed in seeds.clone() {
            let walk: Vec<_> = seeded_walk(&graph, strategy.as_ref(), seed, RecipeId(0), 5)
                .take(steps)
                .collect();
            let report = variety(&walk, &categories, window);
            repeat += report.immediate_repeat_rate;
            distinct += report.distinct_recipe_ratio;
            cats += report.mean_distinct_categories;
            entropy += report.category_entropy;
            runs += 1.0;
        }
        println!(
            "{:<10} {:>8.3} {:>10.3} {:>10.3} {:>9.3}",
            name,
            repeat / runs,
            distinct / runs,
            cats / runs,
            entropy / runs
        );
    }
    println!("\nlower repeat = less clustering; higher cats/win & entropy = more variety");
}
