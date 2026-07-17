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
use recipe_walk::{FixtureGraph, IngredientId, RecipeGraph, RecipeId, TabuWeighted, Walk};
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
/// How many random starts to try, keeping the longest walk. Starts are already
/// chosen from *connected* recipes (see [`Corpus::connected_starts`]), so every
/// attempt journeys; the retries are only to avoid returning a short cul-de-sac
/// when a different start would have wandered further.
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

    /// Recipes that can actually begin a journey: those with at least one
    /// ingredient shared by another recipe (frequency ≥ 2). Starting here means the
    /// first hop always exists, so a walk only comes up short in a genuinely sparse
    /// corpus — never because it happened to begin on an island (a recipe whose
    /// ingredients are all unique to it). Empty only if *no* recipe shares any
    /// ingredient with another at all.
    fn connected_starts(&self) -> Vec<RecipeId> {
        (0..self.cards.len() as u32)
            .map(RecipeId)
            .filter(|&r| {
                self.graph
                    .ingredients_of(r)
                    .iter()
                    .any(|&i| self.graph.frequency(i) >= 2)
            })
            .collect()
    }
}

/// Produce a walk of up to `len` stops over `corpus`, using the caller's `rng` for
/// both the starting recipe and every hop.
///
/// Pure over `(corpus, rng)` so a seeded rng makes it deterministic to test.
/// Starts from a *connected* recipe (one with a shared ingredient) and tries a few
/// of them, returning the longest walk found. Empty corpus → no stops.
fn wander<R: RngCore>(corpus: &Corpus, len: usize, rng: &mut R) -> Vec<Stop> {
    if corpus.len() == 0 {
        return Vec::new();
    }
    let strategy = TabuWeighted::default();

    // Begin only where a hop is possible. If the corpus is so sparse that no
    // recipe shares an ingredient, fall back to any recipe — the walk will be a
    // single lonely stop, which is the honest answer for a corpus with no edges.
    let starts = corpus.connected_starts();
    let pool: Vec<RecipeId> = if starts.is_empty() {
        (0..corpus.len() as u32).map(RecipeId).collect()
    } else {
        starts
    };

    let mut best: Vec<Stop> = Vec::new();
    for _ in 0..START_ATTEMPTS {
        let start = pool[rng.gen_range(0..pool.len())];
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
        // The ingredients column is our own serialization — NOT NULL DEFAULT '[]',
        // written only by ingest — so the two ways to fail here are not the same.
        // A column-read error is *structural*: the column is gone or the wrong
        // type, which is schema drift affecting every row, so it propagates and
        // fails the request loudly, the way a wrong DATABASE_URL does (see db.rs).
        // A JSON *parse* error is per-row: one corrupt value must not 500 a walk
        // that works over the other recipes, so that recipe degrades to an
        // edgeless node — but it is warned, not dropped silently, so corruption is
        // still visible.
        let json = row.get::<String>(6)?;
        let ingredients: Vec<Ingredient> = serde_json::from_str(&json).unwrap_or_else(|e| {
            tracing::warn!(
                "recipe {}/{} has unparseable ingredients JSON, treating as none: {e}",
                card.source,
                card.id
            );
            Vec::new()
        });
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
        assert_eq!(corpus.graph.recipes_with(miso), &[RecipeId(0), RecipeId(1)]);
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

    /// An island (a recipe whose ingredients are all unique to it) shares
    /// `unobtanium` with nobody, so it can never begin a journey; the connected
    /// trio can.
    fn island_and_trio() -> Corpus {
        Corpus::build(vec![
            row("0", "lonely", &["unobtanium"]),
            row("1", "A", &["shared", "a"]),
            row("2", "B", &["shared", "b"]),
            row("3", "C", &["shared", "c"]),
        ])
    }

    #[test]
    fn connected_starts_excludes_islands() {
        let corpus = island_and_trio();
        // `shared` has frequency 3; every other ingredient is frequency 1. So only
        // the trio can start a walk — recipe 0 is left out.
        assert_eq!(
            corpus.connected_starts(),
            vec![RecipeId(1), RecipeId(2), RecipeId(3)]
        );
    }

    #[test]
    fn an_island_is_never_the_start_of_a_journey() {
        // For every seed, the walk begins in the connected trio, never on the
        // lonely recipe — exactly what `connected_starts` guarantees.
        //
        // It does NOT guarantee a *long* walk here: TabuWeighted favours
        // distinctive (rare) ingredients, and in this tiny fixture the rare ones
        // (a/b/c) are dead ends while the connective `shared` is disfavoured, so a
        // start can still dead-end at one stop. That is a fine "up to N" outcome
        // and a non-issue on a real corpus where recipes share many ingredients.
        // The guarantee here is only about *where a walk starts*; that a dense
        // corpus actually journeys is covered by `every_stop_is_reachable_by_its_via`.
        let corpus = island_and_trio();
        for seed in 0..16 {
            let mut rng = StdRng::seed_from_u64(seed);
            let stops = wander(&corpus, 6, &mut rng);
            assert!(
                !stops.is_empty(),
                "a non-empty corpus yields a stop (seed {seed})"
            );
            assert_ne!(
                stops[0].recipe.id, "0",
                "the island is never a start (seed {seed})"
            );
        }
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
