//! # recipe-walk
//!
//! A variety-first **walk** over the recipe↔ingredient graph — the engine behind
//! `pick` (see the issue). It *wanders* recipe space instead of searching it:
//! from a recipe, hop to an ingredient, then to another recipe that shares it,
//! and keep going. The ingredient you crossed is the thread ("via miso"), so a
//! walk reads as a journey rather than a shuffle.
//!
//! ## Two traits, one of which you experiment with
//!
//! Everything runs over [`RecipeGraph`] — the corpus, abstracted. In production
//! that is the normalized `recipes` view in Turso; in tests and experiments it is
//! an in-memory [`FixtureGraph`]. The walk never touches a database, a remote, or
//! `raw_imports` directly.
//!
//! The one thing that changes over time is the *next* decision, behind
//! [`NextStep`]. Swapping the strategy is swapping one value; the [`Walk`] driver
//! never moves. The strategies live in [`strategy`], the offline variety metrics
//! in [`eval`].
//!
//! ## Why the naïve version fails
//!
//! A uniform walk concentrates on hubs, and the hubs are proteins and staples —
//! hop via `chicken` and the next recipe is another chicken dish. The fix is
//! *which* ingredient you hop by: favour the distinctive, mid-frequency
//! ingredients that bridge categories, and don't re-cross one you just used. That
//! is [`strategy::DistinctivenessWeighted`] and [`strategy::TabuWeighted`]; the
//! naïve [`strategy::UniformWalk`] is kept as the control to measure against.
//!
//! ## IDs are opaque
//!
//! The walk works over integer handles, not recipe models — so it is fast and
//! decoupled. Whoever builds the [`RecipeGraph`] (the Turso reader, a fixture)
//! owns the mapping from real recipe/ingredient identifiers to these handles.

use std::collections::VecDeque;

use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};

pub mod eval;
pub mod graph;
pub mod strategy;

pub use graph::{FixtureGraph, RecipeGraph};
pub use strategy::{DistinctivenessWeighted, NextStep, TabuWeighted, UniformWalk};

/// An opaque handle to a recipe node. Its meaning is owned by the [`RecipeGraph`]
/// that produced it (a row in the normalized corpus, an index in a fixture).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecipeId(pub u32);

/// An opaque handle to an ingredient node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IngredientId(pub u32);

/// One hop of the walk: the recipe arrived at, and the ingredient crossed to
/// reach it. `via` is the bridge — the thread the UI shows so wandering has a
/// story ("… → via miso → miso aubergine").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub via: IngredientId,
    pub recipe: RecipeId,
}

/// Where the walk is, and the short memory of where it just was. Strategies read
/// this to keep the walk from oscillating; the [`Walk`] driver maintains it.
///
/// `window` is the tabu horizon — how many recent recipes and ingredients count
/// as "just used". A window of 0 disables the memory (every step is amnesiac).
pub struct WalkState {
    current: RecipeId,
    recent_recipes: VecDeque<RecipeId>,
    recent_ingredients: VecDeque<IngredientId>,
    window: usize,
}

impl WalkState {
    /// A fresh state at `start` with a tabu horizon of `window` steps.
    pub fn new(start: RecipeId, window: usize) -> Self {
        Self {
            current: start,
            recent_recipes: VecDeque::new(),
            recent_ingredients: VecDeque::new(),
            window,
        }
    }

    /// The recipe the walk is on.
    pub fn current(&self) -> RecipeId {
        self.current
    }

    /// Was this recipe one of the last `window` visited?
    pub fn recently_visited(&self, recipe: RecipeId) -> bool {
        self.recent_recipes.contains(&recipe)
    }

    /// Did the walk cross this ingredient in the last `window` steps?
    pub fn recently_hopped(&self, ingredient: IngredientId) -> bool {
        self.recent_ingredients.contains(&ingredient)
    }

    /// Advance onto a step: remember where we were, then move.
    fn record(&mut self, step: &Step) {
        if self.window > 0 {
            self.recent_recipes.push_back(self.current);
            self.recent_ingredients.push_back(step.via);
            while self.recent_recipes.len() > self.window {
                self.recent_recipes.pop_front();
            }
            while self.recent_ingredients.len() > self.window {
                self.recent_ingredients.pop_front();
            }
        }
        self.current = step.recipe;
    }
}

/// The driver. Applies a [`NextStep`] over a [`RecipeGraph`], yielding [`Step`]s
/// as an iterator — `walk.take(20)` is a twenty-stop stroll. The interesting
/// logic is entirely in the strategy; this just loops and keeps the memory.
pub struct Walk<'a, R: RngCore> {
    graph: &'a dyn RecipeGraph,
    strategy: &'a dyn NextStep,
    rng: R,
    state: WalkState,
}

impl<'a, R: RngCore> Walk<'a, R> {
    /// Start a walk at `start`, deciding each step with `strategy`, remembering
    /// the last `window` steps.
    pub fn new(
        graph: &'a dyn RecipeGraph,
        strategy: &'a dyn NextStep,
        rng: R,
        start: RecipeId,
        window: usize,
    ) -> Self {
        Self {
            graph,
            strategy,
            rng,
            state: WalkState::new(start, window),
        }
    }

    /// The live walk state (current stop, tabu memory).
    pub fn state(&self) -> &WalkState {
        &self.state
    }
}

impl<R: RngCore> Iterator for Walk<'_, R> {
    type Item = Step;

    fn next(&mut self) -> Option<Step> {
        let step = self.strategy.next(&self.state, self.graph, &mut self.rng)?;
        self.state.record(&step);
        Some(step)
    }
}

/// A reproducible walk: the same `seed` yields the same sequence of steps over
/// the same graph. Reproducibility is the whole point of a seed here — it is how
/// two strategies get compared on identical conditions, and how tests stay
/// deterministic.
pub fn seeded_walk<'a>(
    graph: &'a dyn RecipeGraph,
    strategy: &'a dyn NextStep,
    seed: u64,
    start: RecipeId,
    window: usize,
) -> Walk<'a, StdRng> {
    Walk::new(graph, strategy, StdRng::seed_from_u64(seed), start, window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_state_tabu_horizon() {
        let mut s = WalkState::new(RecipeId(0), 2);
        s.record(&Step {
            via: IngredientId(10),
            recipe: RecipeId(1),
        });
        s.record(&Step {
            via: IngredientId(11),
            recipe: RecipeId(2),
        });
        assert_eq!(s.current(), RecipeId(2));
        // The two most recent are remembered.
        assert!(s.recently_visited(RecipeId(0)));
        assert!(s.recently_visited(RecipeId(1)));
        assert!(s.recently_hopped(IngredientId(10)));
        // A third step pushes the oldest out of the horizon.
        s.record(&Step {
            via: IngredientId(12),
            recipe: RecipeId(3),
        });
        assert!(!s.recently_visited(RecipeId(0)));
        assert!(!s.recently_hopped(IngredientId(10)));
        assert!(s.recently_visited(RecipeId(2)));
    }

    #[test]
    fn zero_window_is_amnesiac() {
        let mut s = WalkState::new(RecipeId(0), 0);
        s.record(&Step {
            via: IngredientId(1),
            recipe: RecipeId(1),
        });
        assert!(!s.recently_visited(RecipeId(0)));
        assert!(!s.recently_hopped(IngredientId(1)));
        assert_eq!(s.current(), RecipeId(1));
    }

    #[test]
    fn same_seed_same_walk() {
        // A tiny triangle where every ingredient joins two recipes.
        let g = FixtureGraph::new(vec![
            vec![IngredientId(0), IngredientId(1)],
            vec![IngredientId(1), IngredientId(2)],
            vec![IngredientId(2), IngredientId(0)],
        ]);
        let strat = UniformWalk;
        let a: Vec<_> = seeded_walk(&g, &strat, 42, RecipeId(0), 3)
            .take(30)
            .collect();
        let b: Vec<_> = seeded_walk(&g, &strat, 42, RecipeId(0), 3)
            .take(30)
            .collect();
        let c: Vec<_> = seeded_walk(&g, &strat, 7, RecipeId(0), 3)
            .take(30)
            .collect();
        assert_eq!(a, b, "same seed must reproduce the walk");
        assert_ne!(a, c, "a different seed should (almost surely) differ");
    }
}
