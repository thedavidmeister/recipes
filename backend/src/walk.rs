//! `GET /api/walk` — a variety-first wander over the corpus (#47).
//!
//! The walk *wanders* recipe space instead of searching it: from a recipe, hop to
//! one of its ingredients, then to another recipe that shares it, and keep going.
//! The ingredient crossed is the thread the UI shows ("… → via miso → miso
//! aubergine"), so a walk reads as a journey rather than a shuffle. The decision
//! logic lives in the `recipe-walk` crate behind [`recipe_walk::NextStep`]; this
//! module only builds the graph the walk runs over and turns its opaque steps
//! back into recipes the client can render.
//!
//! **Corpus only, never remotes** (#47): the graph is built from the normalized
//! `recipes` view in Turso — already-ingested, already-derived rows. A step is a
//! local lookup; the walk never fetches a source and never widens the corpus (that
//! is ingest's job). It is a reader.
//!
//! **The graph is loaded once per request, not queried per hop.** The walk makes
//! many tight `ingredients_of` / `recipes_with` calls, so a graph that hit Turso
//! on each would be pathological. Instead one query loads the corpus and
//! [`recipe_walk::FixtureGraph`] indexes it in memory for the life of the request
//! — the same in-memory bipartite index the crate uses offline, which is exactly
//! what a hot walk loop needs. There is no persistent cache (see CLAUDE.md): a
//! fresh load per request is cheap at this corpus size and always current after an
//! ingest.
//!
//! **Ingredient nodes are names, normalized by case/whitespace.** TheMealDB already
//! separates an ingredient's name from its measure, so its names are node-quality
//! today; #11 (structured ingredients) sharpens this for free-text sources and
//! near-duplicate names, but the walk does not wait on it for the corpus we hold.

use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    Json,
};
use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};
use recipe_core::Ingredient;
use recipe_walk::{FixtureGraph, IngredientId, RecipeId, TabuWeighted, Walk};
use serde::{Deserialize, Serialize};

use crate::{error::AppError, AppState};

/// How many stops a walk has when the caller does not say. Long enough to feel
/// like a journey, short enough to render at a glance.
const DEFAULT_LEN: usize = 12;
/// A ceiling on `len`, so a caller cannot ask for an unbounded walk.
const MAX_LEN: usize = 30;
/// The tabu horizon: how many recent recipes/ingredients the walk refuses to
/// re-cross. Comfortably larger than a default walk so it does not oscillate; the
/// strategy relaxes it rather than dead-ending if a corner is that tight.
const TABU_WINDOW: usize = 12;
/// How many random starts to try before giving up on a longer walk. A start on an
/// isolated recipe (no ingredient shared with another) walks nowhere; the corpus
/// is dense, so a couple of retries all but guarantees a real journey without
/// scanning for a "good" start.
const START_ATTEMPTS: usize = 8;

/// Query string for `GET /api/walk`.
#[derive(Debug, Deserialize)]
pub struct WalkParams {
    /// Requested number of stops, clamped to `1..=MAX_LEN`. Absent → [`DEFAULT_LEN`].
    len: Option<usize>,
}

/// What the client needs to render one stop — the read fields of a recipe, no
/// ingredients or instructions (a card, not the full page).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecipeCard {
    pub source: String,
    pub id: String,
    pub title: String,
    pub image: Option<String>,
    pub category: Option<String>,
    pub area: Option<String>,
}

/// One stop on the walk: the recipe landed on, and the ingredient crossed to reach
/// it. `via` is `None` only for the first stop — the walk's starting point, which
/// was arrived at by nothing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stop {
    pub via: Option<String>,
    pub recipe: RecipeCard,
}

/// The whole journey, in order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WalkResponse {
    pub stops: Vec<Stop>,
}

/// The corpus as the walk sees it: an in-memory bipartite index plus the mappings
/// from the walk's opaque ids back to renderable data. Built once per request from
/// loaded rows.
struct Corpus {
    graph: FixtureGraph,
    /// `RecipeId(i)` → its card.
    cards: Vec<RecipeCard>,
    /// `IngredientId(i)` → the name to show for the thread ("via …").
    ingredient_names: Vec<String>,
}

impl Corpus {
    /// Build from `(card, ingredient names)` rows. Ingredient nodes are interned
    /// by a normalized key (trimmed, lowercased) so `"Miso"` and `"miso"` are one
    /// node; the first spelling seen is kept for display. Blank names are dropped —
    /// they are not ingredients and would fuse unrelated recipes into one hub.
    fn build(rows: Vec<(RecipeCard, Vec<String>)>) -> Self {
        let mut ids: HashMap<String, IngredientId> = HashMap::new();
        let mut ingredient_names: Vec<String> = Vec::new();
        let mut by_recipe: Vec<Vec<IngredientId>> = Vec::with_capacity(rows.len());
        let mut cards: Vec<RecipeCard> = Vec::with_capacity(rows.len());

        for (card, names) in rows {
            let mut list = Vec::new();
            for name in names {
                let key = name.trim().to_lowercase();
                if key.is_empty() {
                    continue;
                }
                let id = *ids.entry(key).or_insert_with(|| {
                    let id = IngredientId(ingredient_names.len() as u32);
                    ingredient_names.push(name.trim().to_string());
                    id
                });
                // A recipe listing the same ingredient twice is one edge, not two.
                if !list.contains(&id) {
                    list.push(id);
                }
            }
            by_recipe.push(list);
            cards.push(card);
        }

        Corpus {
            graph: FixtureGraph::new(by_recipe),
            cards,
            ingredient_names,
        }
    }

    /// Number of recipes in the corpus.
    fn len(&self) -> usize {
        self.cards.len()
    }
}

/// Produce a walk of up to `len` stops over `corpus`, using the caller's `rng` for
/// both the starting recipe and every hop.
///
/// Pure over `(corpus, rng)` so a seeded rng makes it deterministic to test. Tries
/// a few random starts so an isolated recipe does not yield a one-stop journey;
/// returns the longest walk found. Empty corpus → no stops.
fn wander<R: RngCore>(corpus: &Corpus, len: usize, rng: &mut R) -> Vec<Stop> {
    if corpus.len() == 0 {
        return Vec::new();
    }
    let strategy = TabuWeighted::default();

    let mut best: Vec<Stop> = Vec::new();
    for _ in 0..START_ATTEMPTS {
        let start = RecipeId(rng.gen_range(0..corpus.len() as u32));
        let mut stops = vec![Stop {
            via: None,
            recipe: corpus.cards[start.0 as usize].clone(),
        }];
        // `Walk` needs an owned rng; reborrow the shared one through a thin adapter
        // so the same stream drives start selection and hops.
        let walk = Walk::new(&corpus.graph, &strategy, &mut *rng, start, TABU_WINDOW);
        for step in walk.take(len.saturating_sub(1)) {
            stops.push(Stop {
                via: corpus.ingredient_names.get(step.via.0 as usize).cloned(),
                recipe: corpus.cards[step.recipe.0 as usize].clone(),
            });
        }
        if stops.len() > best.len() {
            best = stops;
        }
        // A full-length walk is as good as it gets — stop early.
        if best.len() >= len {
            break;
        }
    }
    best
}

/// Load the whole normalized corpus into a [`Corpus`]. One query; the ingredients
/// column is JSON, parsed here into names (measures are irrelevant to the graph).
async fn load_corpus(conn: &libsql::Connection) -> anyhow::Result<Corpus> {
    let mut rows = conn
        .query(
            "SELECT source, id, title, image, category, area, ingredients
             FROM recipes",
            (),
        )
        .await?;

    let mut out: Vec<(RecipeCard, Vec<String>)> = Vec::new();
    while let Some(row) = rows.next().await? {
        let card = RecipeCard {
            source: row.get::<String>(0)?,
            id: row.get::<String>(1)?,
            title: row.get::<String>(2)?,
            image: row.get::<Option<String>>(3)?,
            category: row.get::<Option<String>>(4)?,
            area: row.get::<Option<String>>(5)?,
        };
        // Ingredients are stored as a JSON array of {name, measure}. A malformed
        // or empty column just means "no edges from this recipe" — one lonely
        // node, not a failed request.
        let ingredients: Vec<Ingredient> = row
            .get::<String>(6)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        let names = ingredients.into_iter().map(|i| i.name).collect();
        out.push((card, names));
    }

    Ok(Corpus::build(out))
}

/// `GET /api/walk?len=<n>` — a fresh variety-first walk over the corpus.
///
/// Session-gated like every person-facing route (#25). Each call re-seeds from OS
/// entropy, so the same corpus yields a different journey every time — freshness is
/// the whole point (#47).
pub async fn walk(
    State(state): State<AppState>,
    Query(params): Query<WalkParams>,
) -> Result<Json<WalkResponse>, AppError> {
    let len = params.len.unwrap_or(DEFAULT_LEN).clamp(1, MAX_LEN);
    let corpus = load_corpus(&state.db)
        .await
        .map_err(|e| AppError::Internal(format!("could not load the corpus: {e}")))?;
    let mut rng = StdRng::from_entropy();
    let stops = wander(&corpus, len, &mut rng);
    Ok(Json(WalkResponse { stops }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_walk::RecipeGraph;

    fn card(id: &str, title: &str) -> RecipeCard {
        RecipeCard {
            source: "test".into(),
            id: id.into(),
            title: title.into(),
            image: None,
            category: None,
            area: None,
        }
    }

    fn row(id: &str, title: &str, ingredients: &[&str]) -> (RecipeCard, Vec<String>) {
        (
            card(id, title),
            ingredients.iter().map(|s| s.to_string()).collect(),
        )
    }

    /// A corpus where every recipe shares one ingredient with the next, so a walk
    /// can always move — a ring, no dead ends.
    fn ring_corpus(n: usize) -> Corpus {
        let rows = (0..n)
            .map(|r| {
                let here = format!("ing{r}");
                let prev = format!("ing{}", (r + n - 1) % n);
                (
                    card(&r.to_string(), &format!("recipe {r}")),
                    vec![here, prev],
                )
            })
            .collect();
        Corpus::build(rows)
    }

    #[test]
    fn ingredient_nodes_are_normalized_and_deduped() {
        let corpus = Corpus::build(vec![
            row("1", "A", &["Miso", "Chicken"]),
            row("2", "B", &["miso ", " CHICKEN"]),
        ]);
        // "Miso"/"miso " and "Chicken"/" CHICKEN" each collapse to one node.
        assert_eq!(corpus.ingredient_names.len(), 2);
        // Both recipes share both ingredients, so each ingredient joins both.
        let miso = corpus.graph.ingredients_of(RecipeId(0))[0];
        assert_eq!(
            recipe_walk::RecipeGraph::recipes_with(&corpus.graph, miso),
            &[RecipeId(0), RecipeId(1)]
        );
    }

    #[test]
    fn blank_ingredient_names_are_dropped() {
        let corpus = Corpus::build(vec![row("1", "A", &["", "  ", "salt"])]);
        assert_eq!(corpus.ingredient_names, vec!["salt"]);
    }

    #[test]
    fn a_recipe_listing_one_ingredient_twice_makes_one_edge() {
        let corpus = Corpus::build(vec![row("1", "A", &["salt", "Salt"])]);
        assert_eq!(corpus.graph.ingredients_of(RecipeId(0)).len(), 1);
    }

    #[test]
    fn empty_corpus_yields_no_stops() {
        let corpus = Corpus::build(vec![]);
        let mut rng = StdRng::seed_from_u64(1);
        assert!(wander(&corpus, 12, &mut rng).is_empty());
    }

    #[test]
    fn every_stop_is_reachable_by_its_via() {
        // A ring of 20 recipes: a walk should produce a legal, connected journey.
        let corpus = ring_corpus(20);
        let mut rng = StdRng::seed_from_u64(7);
        let stops = wander(&corpus, 12, &mut rng);
        assert_eq!(stops.len(), 12, "a dense corpus walks the full length");
        assert!(
            stops[0].via.is_none(),
            "the first stop is arrived at by nothing"
        );

        for pair in stops.windows(2) {
            let via = pair[1]
                .via
                .as_ref()
                .expect("only the first stop has no via");
            // The via ingredient must belong to BOTH the previous recipe (hopped
            // from) and this one (hopped to).
            let prev_has = recipe_has(&corpus, &pair[0].recipe, via);
            let here_has = recipe_has(&corpus, &pair[1].recipe, via);
            assert!(prev_has, "via '{via}' must be in the recipe we left");
            assert!(here_has, "via '{via}' must be in the recipe we reached");
        }
    }

    #[test]
    fn a_walk_moves_rather_than_repeating() {
        let corpus = ring_corpus(20);
        let mut rng = StdRng::seed_from_u64(3);
        let stops = wander(&corpus, 12, &mut rng);
        for pair in stops.windows(2) {
            assert_ne!(
                pair[0].recipe.id, pair[1].recipe.id,
                "consecutive stops must be different recipes"
            );
        }
    }

    #[test]
    fn an_isolated_start_still_finds_a_journey() {
        // Recipe 0 shares nothing; recipes 1..=3 form a connected trio. Retried
        // starts should find the trio rather than returning a lonely recipe 0.
        let mut rows = vec![row("0", "lonely", &["unobtanium"])];
        rows.push(row("1", "A", &["shared", "a"]));
        rows.push(row("2", "B", &["shared", "b"]));
        rows.push(row("3", "C", &["shared", "c"]));
        let corpus = Corpus::build(rows);
        // A seed that would start on the lonely recipe first still yields >1 stop.
        let mut found_long = false;
        for seed in 0..8 {
            let mut rng = StdRng::seed_from_u64(seed);
            if wander(&corpus, 6, &mut rng).len() > 1 {
                found_long = true;
                break;
            }
        }
        assert!(found_long, "retried starts must escape an isolated recipe");
    }

    /// Does `card`'s recipe list `via` (by the same normalization the graph uses)?
    fn recipe_has(corpus: &Corpus, target: &RecipeCard, via: &str) -> bool {
        let key = via.trim().to_lowercase();
        let Some(idx) = corpus.cards.iter().position(|c| c.id == target.id) else {
            return false;
        };
        corpus
            .graph
            .ingredients_of(RecipeId(idx as u32))
            .iter()
            .any(|&i| corpus.ingredient_names[i.0 as usize].trim().to_lowercase() == key)
    }
}
