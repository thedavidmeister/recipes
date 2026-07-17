//! The *next* decision — the one thing that changes over time.
//!
//! Each strategy is a [`NextStep`]: given where the walk is (and its short
//! memory), pick the ingredient to hop by and the recipe to land on. Swapping
//! strategies is swapping one value passed to [`crate::Walk`].
//!
//! Three are provided, in ascending order of "keeps the walk fresh":
//!
//! - [`UniformWalk`] — the naïve baseline. Kept as the control: it is expected to
//!   cluster (hop via hubs like a main protein and stay in that category), and
//!   the whole point is to measure the others against it.
//! - [`DistinctivenessWeighted`] — bias the ingredient hop toward *distinctive*
//!   (low-frequency) ingredients. A hub like salt is a near-random teleport and a
//!   protein is a category trap; a mid-frequency bridge (miso, cumin) crosses
//!   categories while staying flavour-coherent.
//! - [`TabuWeighted`] — distinctiveness plus the recent-memory penalties: don't
//!   re-cross an ingredient you just used, don't land back on a recent recipe.
//!
//! ## Dead ends
//!
//! Every strategy hops only by a *viable* ingredient — one another recipe also
//! lists (see [`viable_ingredients`]). An ingredient unique to the current recipe
//! leads nowhere, and since a distinctiveness bias actively favours the rarest
//! ingredients — a unique one most of all — choosing without this filter would
//! keep stranding the walk one hop in. Filtering first means a strategy only
//! returns `None` when the recipe has *no* onward ingredient at all — a true dead
//! end — not because it happened to pick the one ingredient that goes nowhere.

use rand::distributions::{Distribution, WeightedIndex};
use rand::seq::SliceRandom;
use rand::RngCore;

use crate::graph::RecipeGraph;
use crate::{IngredientId, RecipeId, Step, WalkState};

/// How a walk chooses its next step. This is the pluggable surface — everything
/// else in the crate is fixed scaffolding around it.
///
/// Returns `None` at a dead end (the current recipe has no ingredient that leads
/// anywhere else). The trait is object-safe so strategies can be stored and
/// swapped as `&dyn NextStep`.
pub trait NextStep {
    fn next(
        &self,
        state: &WalkState,
        graph: &dyn RecipeGraph,
        rng: &mut dyn RngCore,
    ) -> Option<Step>;
}

/// The naïve baseline: a uniform hop. Pick any ingredient of the current recipe,
/// then any other recipe that has it. Expected to cluster on hubs — kept as the
/// control the others are measured against.
pub struct UniformWalk;

impl NextStep for UniformWalk {
    fn next(
        &self,
        state: &WalkState,
        graph: &dyn RecipeGraph,
        rng: &mut dyn RngCore,
    ) -> Option<Step> {
        let viable = viable_ingredients(state.current(), graph);
        let &via = viable.choose(rng)?;
        let recipe = choose_recipe(graph, via, state.current(), &|_| false, rng)?;
        Some(Step { via, recipe })
    }
}

/// Hop by *distinctive* ingredients. Weight each ingredient by `1 / freq^strength`
/// so low-frequency bridges dominate the choice over commodity hubs. `strength`
/// tunes the bias: `0.0` is uniform, `1.0` is inverse frequency, higher is
/// sharper. The recipe itself is still picked uniformly from the bridge's pool.
pub struct DistinctivenessWeighted {
    pub strength: f64,
}

impl Default for DistinctivenessWeighted {
    fn default() -> Self {
        Self { strength: 1.0 }
    }
}

impl NextStep for DistinctivenessWeighted {
    fn next(
        &self,
        state: &WalkState,
        graph: &dyn RecipeGraph,
        rng: &mut dyn RngCore,
    ) -> Option<Step> {
        let viable = viable_ingredients(state.current(), graph);
        let via = weighted_ingredient(&viable, graph, self.strength, &|_| false, rng)?;
        let recipe = choose_recipe(graph, via, state.current(), &|_| false, rng)?;
        Some(Step { via, recipe })
    }
}

/// Distinctiveness, plus don't double back. Deprioritise ingredients crossed in
/// the last few steps, and avoid landing on recently-visited recipes — so the
/// walk actively spreads instead of oscillating between a couple of stops. Both
/// penalties relax rather than dead-end: if every option is tabu, it takes the
/// least-bad one rather than stopping.
pub struct TabuWeighted {
    pub strength: f64,
}

impl Default for TabuWeighted {
    fn default() -> Self {
        Self { strength: 1.0 }
    }
}

impl NextStep for TabuWeighted {
    fn next(
        &self,
        state: &WalkState,
        graph: &dyn RecipeGraph,
        rng: &mut dyn RngCore,
    ) -> Option<Step> {
        let viable = viable_ingredients(state.current(), graph);
        let via = weighted_ingredient(
            &viable,
            graph,
            self.strength,
            &|i| state.recently_hopped(i),
            rng,
        )?;
        let recipe = choose_recipe(
            graph,
            via,
            state.current(),
            &|r| state.recently_visited(r),
            rng,
        )?;
        Some(Step { via, recipe })
    }
}

/// The current recipe's ingredients that actually lead somewhere new — those some
/// *other* recipe also lists. Hopping by an ingredient unique to this recipe
/// dead-ends the walk, so every strategy chooses only from these; when this is
/// empty the recipe is a genuine dead end and the walk stops there.
///
/// This is what keeps a distinctiveness bias honest. That bias favours *rare*
/// ingredients, and the rarest of all is one unique to a single recipe — a dead
/// end. Without this filter a strategy would keep picking exactly those and strand
/// the walk one hop in; with it, "prefer the distinctive" means "prefer the
/// distinctive *bridge*", which is the whole point.
fn viable_ingredients(current: RecipeId, graph: &dyn RecipeGraph) -> Vec<IngredientId> {
    graph
        .ingredients_of(current)
        .iter()
        .copied()
        .filter(|&i| graph.recipes_with(i).iter().any(|&r| r != current))
        .collect()
}

/// Pick an ingredient from `ingredients`, weighted by distinctiveness, skipping
/// any the caller vetoes. If the veto zeroes everything, it is dropped (better a
/// tabu hop than a stuck walk). `None` only when there are no ingredients at all.
fn weighted_ingredient(
    ingredients: &[IngredientId],
    graph: &dyn RecipeGraph,
    strength: f64,
    veto: &dyn Fn(IngredientId) -> bool,
    rng: &mut dyn RngCore,
) -> Option<IngredientId> {
    if ingredients.is_empty() {
        return None;
    }
    let weight = |i: IngredientId| -> f64 {
        let freq = graph.frequency(i).max(1) as f64;
        1.0 / freq.powf(strength)
    };
    let vetoed: Vec<f64> = ingredients
        .iter()
        .map(|&i| if veto(i) { 0.0 } else { weight(i) })
        .collect();
    // WeightedIndex fails if every weight is zero — fall back to ignoring the veto.
    let dist = WeightedIndex::new(&vetoed)
        .or_else(|_| WeightedIndex::new(ingredients.iter().map(|&i| weight(i)).collect::<Vec<_>>()))
        .ok()?;
    Some(ingredients[dist.sample(rng)])
}

/// Pick a recipe that contains `via`, never the current one, preferring those the
/// caller does not veto. If the veto leaves nothing, it relaxes to any non-current
/// recipe. `None` only when `via` leads nowhere but back to `current`.
fn choose_recipe(
    graph: &dyn RecipeGraph,
    via: IngredientId,
    current: RecipeId,
    veto: &dyn Fn(RecipeId) -> bool,
    rng: &mut dyn RngCore,
) -> Option<RecipeId> {
    let pool = graph.recipes_with(via);
    let fresh: Vec<RecipeId> = pool
        .iter()
        .copied()
        .filter(|&r| r != current && !veto(r))
        .collect();
    if let Some(&r) = fresh.choose(rng) {
        return Some(r);
    }
    let any: Vec<RecipeId> = pool.iter().copied().filter(|&r| r != current).collect();
    any.choose(rng).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{seeded_walk, FixtureGraph};

    /// Every step a strategy yields must be legal: the recipe contains the bridge
    /// ingredient, and the walk actually moved.
    fn assert_steps_legal(graph: &FixtureGraph, strat: &dyn NextStep) {
        let mut prev = RecipeId(0);
        for step in seeded_walk(graph, strat, 1, RecipeId(0), 4).take(200) {
            assert!(
                graph.recipes_with(step.via).contains(&step.recipe),
                "landed on a recipe that lacks the bridge ingredient"
            );
            assert_ne!(step.recipe, prev, "a step must move off the current recipe");
            prev = step.recipe;
        }
    }

    fn ring(n: u32) -> FixtureGraph {
        // A ring: recipe r shares ingredient r with r+1, so every recipe has two
        // ingredients and two neighbours — no dead ends.
        let recipes = (0..n)
            .map(|r| vec![IngredientId(r), IngredientId((r + n - 1) % n)])
            .collect();
        FixtureGraph::new(recipes)
    }

    #[test]
    fn strategies_yield_legal_steps() {
        let g = ring(12);
        assert_steps_legal(&g, &UniformWalk);
        assert_steps_legal(&g, &DistinctivenessWeighted::default());
        assert_steps_legal(&g, &TabuWeighted::default());
    }

    #[test]
    fn dead_end_stops_the_walk() {
        // Recipe 0's only ingredient belongs to no other recipe.
        let g = FixtureGraph::new(vec![vec![IngredientId(0)]]);
        let mut w = seeded_walk(&g, &UniformWalk, 1, RecipeId(0), 4);
        assert_eq!(w.next(), None);
    }

    /// The dead-end handling: a recipe pairing a rare-but-unique ingredient with a
    /// shared one must not strand the walk. r0 has a dead end (0, unique to it) and
    /// a shared bridge (1); r1 shares 1 and has its own dead end (2). The
    /// distinctiveness bias *prefers* the rare 0 and 2 — precisely the ones that go
    /// nowhere — so a strategy that did not filter would stop one hop in. It must
    /// instead hop by the shared 1 every time and keep going.
    #[test]
    fn a_dead_end_ingredient_does_not_strand_the_walk() {
        let g = FixtureGraph::new(vec![
            vec![IngredientId(0), IngredientId(1)],
            vec![IngredientId(1), IngredientId(2)],
        ]);
        let strategies: [&dyn NextStep; 3] = [
            &UniformWalk,
            &DistinctivenessWeighted::default(),
            &TabuWeighted::default(),
        ];
        for strat in strategies {
            let steps: Vec<_> = seeded_walk(&g, strat, 1, RecipeId(0), 4).take(10).collect();
            assert_eq!(
                steps.len(),
                10,
                "must not dead-end on a rare-but-unique ingredient"
            );
            for s in &steps {
                assert_eq!(
                    s.via,
                    IngredientId(1),
                    "the only viable hop is the shared ingredient"
                );
            }
        }
    }
}
