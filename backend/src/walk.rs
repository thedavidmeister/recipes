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
    /// node; the first spelling seen is kept for display. Names with no letter or
    /// digit are dropped — they are not ingredients and would fuse unrelated recipes
    /// into one hub. That is a stronger test than "blank after trim": `trim` only
    /// removes Unicode *whitespace*, so a zero-width space (U+200B), a word joiner,
    /// or a BOM would otherwise slip through as a real (invisible) node.
    fn build(rows: Vec<(RecipeCard, Vec<String>)>) -> Self {
        let mut ids: HashMap<String, IngredientId> = HashMap::new();
        let mut ingredient_names: Vec<String> = Vec::new();
        let mut by_recipe: Vec<Vec<IngredientId>> = Vec::with_capacity(rows.len());
        let mut cards: Vec<RecipeCard> = Vec::with_capacity(rows.len());

        for (card, names) in rows {
            let mut list = Vec::new();
            for name in names {
                let key = name.trim().to_lowercase();
                // A real ingredient name has at least one letter or digit; anything
                // else (blank, punctuation, invisible formatting characters) is not a
                // node.
                if !key.chars().any(char::is_alphanumeric) {
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

/// Compose a walk of up to `len` **distinct** recipes over `corpus`.
///
/// This is the journey-assembly layer above the per-step strategy. A *self-avoiding*
/// [`Walk`] wanders one connected region, hopping only by an ingredient that leads
/// somewhere unvisited — so it never repeats, and it reports a dead end only when
/// the region's whole reachable frontier is spent (not one hop early because it
/// happened to pick a via whose landings were all seen). When that frontier is
/// spent and more stops are wanted, we **teleport**: jump to a fresh recipe
/// (preferring a connected one, so the new leg can wander) and carry on. A teleport
/// stop has no `via` — it is a new thread, like the very first stop.
///
/// The result is `len` distinct recipes, or every recipe in a corpus that holds
/// fewer than `len` — never a repeat, never trapped, and teleporting only when it
/// genuinely must. Pure over `(corpus, rng)` so a seeded rng makes it deterministic
/// to test. Empty corpus → no stops.
fn wander<R: RngCore>(corpus: &Corpus, len: usize, rng: &mut R) -> Vec<Stop> {
    if corpus.len() == 0 {
        return Vec::new();
    }
    let strategy = TabuWeighted::default();

    // Teleport candidates: connected recipes (a leg can actually wander from them).
    // Fall back to every recipe only if nothing is connected, so an edgeless corpus
    // still yields stops rather than nothing.
    let connected = corpus.connected_starts();
    let all: Vec<RecipeId> = (0..corpus.len() as u32).map(RecipeId).collect();
    let start_pool: &[RecipeId] = if connected.is_empty() {
        &all
    } else {
        &connected
    };

    // The first start, then a self-avoiding walk that owns the visited set. `&mut
    // *rng` reborrows the caller's stream so the start, every hop, and every
    // teleport all draw from the one sequence — a whole journey deterministic in one
    // seed.
    let start = start_pool[rng.gen_range(0..start_pool.len())];
    let mut stops = vec![Stop {
        via: None,
        recipe: corpus.cards[start.0 as usize].clone(),
    }];
    let mut walk = Walk::self_avoiding(&corpus.graph, &strategy, &mut *rng, start, len);

    while stops.len() < len {
        // Wander until this region's frontier is spent (the walk yields `None`).
        while stops.len() < len {
            let Some(step) = walk.next() else { break };
            stops.push(Stop {
                via: corpus.ingredient_names.get(step.via.0 as usize).cloned(),
                recipe: corpus.cards[step.recipe.0 as usize].clone(),
            });
        }
        if stops.len() >= len {
            break;
        }
        // Frontier spent → teleport to a fresh recipe (connected if any remain),
        // starting a new thread. None left → the corpus is exhausted (fewer than
        // `len` recipes), which is the honest answer.
        let fresh = match walk.teleport_to_fresh(start_pool) {
            Some(r) => Some(r),
            None => walk.teleport_to_fresh(&all),
        };
        let Some(fresh) = fresh else { break };
        stops.push(Stop {
            via: None,
            recipe: corpus.cards[fresh.0 as usize].clone(),
        });
    }
    stops
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
    let corpus = load_corpus(&state.db()?)
        .await
        .map_err(|e| AppError::Internal(format!("could not load the corpus: {e}")))?;
    let mut rng = StdRng::from_entropy();
    let stops = wander(&corpus, len, &mut rng);
    Ok(Json(WalkResponse { stops }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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
            // A `None` via is a teleport (a new leg), not a hop — nothing to check.
            // A ring never exhausts within 12 of 20, so this loop sees only hops
            // here, but the walk permits teleports in general.
            let Some(via) = pair[1].via.as_ref() else {
                continue;
            };
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
    fn a_walk_visits_distinct_recipes_and_only_teleports_to_an_island() {
        // Four recipes, asked for six: the walk returns all four, each distinct, no
        // repeats — the corpus simply holds fewer than `len`. The trio is wandered
        // by its shared ingredient; the island (nothing shares `unobtanium`) can
        // only be *teleported* to, never *hopped* to, so whenever it appears it has
        // no `via`.
        let corpus = island_and_trio();
        for seed in 0..16 {
            let mut rng = StdRng::seed_from_u64(seed);
            let stops = wander(&corpus, 6, &mut rng);
            assert_eq!(stops.len(), 4, "all four recipes, no repeats (seed {seed})");
            let distinct: HashSet<_> = stops.iter().map(|s| &s.recipe.id).collect();
            assert_eq!(
                distinct.len(),
                4,
                "every stop is a distinct recipe (seed {seed})"
            );
            for s in &stops {
                if s.recipe.id == "0" {
                    assert!(
                        s.via.is_none(),
                        "the island is only ever a teleport, never hopped to (seed {seed})"
                    );
                }
            }
        }
    }

    #[test]
    fn a_walk_teleports_between_disconnected_regions_for_variety() {
        // Two disconnected trios (each joined by its own `shared_*`). A walk cannot
        // reach six distinct recipes without leaving the first trio, so it must
        // teleport to the second rather than cycle three recipes forever. This is
        // the trap the plain walk fell into; teleporting is what escapes it.
        let corpus = Corpus::build(vec![
            row("0", "A", &["shared_a", "a0"]),
            row("1", "B", &["shared_a", "a1"]),
            row("2", "C", &["shared_a", "a2"]),
            row("3", "D", &["shared_b", "b0"]),
            row("4", "E", &["shared_b", "b1"]),
            row("5", "F", &["shared_b", "b2"]),
        ]);
        for seed in 0..16 {
            let mut rng = StdRng::seed_from_u64(seed);
            let stops = wander(&corpus, 6, &mut rng);
            assert_eq!(
                stops.len(),
                6,
                "six stops across both regions (seed {seed})"
            );
            let distinct: HashSet<_> = stops.iter().map(|s| &s.recipe.id).collect();
            assert_eq!(
                distinct.len(),
                6,
                "no recipe repeats — teleport found fresh ones (seed {seed})"
            );
        }
    }

    #[test]
    fn a_star_corpus_still_gives_distinct_variety() {
        // Every recipe shares one hub ingredient (like salt in the real corpus),
        // plus a unique one. Distinctiveness *disfavours* the hub, but it is the
        // only bridge — so the walk must still hop by it and reach distinct
        // recipes rather than stall on the rare-but-dead-end unique ingredients.
        let rows: Vec<_> = (0..10)
            .map(|r| {
                (
                    card(&r.to_string(), &format!("r{r}")),
                    vec!["hub".to_string(), format!("u{r}")],
                )
            })
            .collect();
        let corpus = Corpus::build(rows);
        for seed in 0..16 {
            let mut rng = StdRng::seed_from_u64(seed);
            let stops = wander(&corpus, 8, &mut rng);
            assert_eq!(stops.len(), 8, "eight of ten reachable (seed {seed})");
            let distinct: HashSet<_> = stops.iter().map(|s| &s.recipe.id).collect();
            assert_eq!(distinct.len(), 8, "distinct despite the hub (seed {seed})");
            // The hub reaches every recipe, so the walk threads all eight by hops:
            // only the start lacks a via, and it never teleports spuriously.
            let teleports = stops.iter().filter(|s| s.via.is_none()).count();
            assert_eq!(
                teleports, 1,
                "a connected corpus needs no teleport (seed {seed})"
            );
        }
    }

    /// The adversarial net: build many random corpora and assert every walk
    /// invariant over many seeds and lengths. If any invariant is breakable, this
    /// finds the (graph, walk, len) that breaks it — a fuzz test in test's
    /// clothing.
    #[test]
    fn wander_invariants_hold_over_random_corpora() {
        for graph_seed in 0..80u64 {
            let mut g = StdRng::seed_from_u64(graph_seed);
            let n_recipes = g.gen_range(0..25usize);
            let n_ingredients = g.gen_range(1..12usize);
            let rows: Vec<_> = (0..n_recipes)
                .map(|r| {
                    // 0..5 ingredients each, drawn from a small shared pool so
                    // components, hubs, islands and dead ends all arise by chance.
                    let k = g.gen_range(0..5usize);
                    let names: Vec<String> = (0..k)
                        .map(|_| format!("ing{}", g.gen_range(0..n_ingredients)))
                        .collect();
                    (card(&r.to_string(), &format!("r{r}")), names)
                })
                .collect();
            let corpus = Corpus::build(rows);

            for walk_seed in 0..12u64 {
                for &len in &[1usize, 2, 5, 12, 30] {
                    let mut rng = StdRng::seed_from_u64(walk_seed.wrapping_mul(31).wrapping_add(1));
                    let stops = wander(&corpus, len, &mut rng);
                    let ctx =
                        format!("graph={graph_seed} walk={walk_seed} len={len} n={n_recipes}");

                    // Deterministic: the same seed reproduces the walk exactly.
                    let mut rng2 =
                        StdRng::seed_from_u64(walk_seed.wrapping_mul(31).wrapping_add(1));
                    assert_eq!(
                        stops,
                        wander(&corpus, len, &mut rng2),
                        "determinism | {ctx}"
                    );

                    // Exactly min(len, corpus size) — no shortfall while fresh
                    // recipes remain, no overshoot.
                    assert_eq!(stops.len(), len.min(corpus.len()), "length | {ctx}");

                    // Every stop is a distinct recipe (ids are unique here).
                    let distinct: HashSet<_> = stops.iter().map(|s| &s.recipe.id).collect();
                    assert_eq!(distinct.len(), stops.len(), "distinct | {ctx}");

                    // The first stop is always a teleport (no via).
                    if let Some(first) = stops.first() {
                        assert!(first.via.is_none(), "first via none | {ctx}");
                    }

                    // Every Some(via) belongs to both adjacent recipes; a None via
                    // is a teleport (a fresh leg, not a hop) and is not checked.
                    for pair in stops.windows(2) {
                        if let Some(via) = pair[1].via.as_ref() {
                            assert!(!via.is_empty(), "via is a real name | {ctx}");
                            assert!(
                                recipe_has(&corpus, &pair[0].recipe, via),
                                "left has via | {ctx}"
                            );
                            assert!(
                                recipe_has(&corpus, &pair[1].recipe, via),
                                "right has via | {ctx}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Does `card`'s recipe list `via` (by the same normalization the graph uses)?
    fn recipe_has(corpus: &Corpus, target: &RecipeCard, via: &str) -> bool {
        let key = via.trim().to_lowercase();
        // Match on the full identity (source, id): two sources can share an id
        // string, and resolving by id alone could check the wrong recipe's row.
        let Some(idx) = corpus
            .cards
            .iter()
            .position(|c| c.source == target.source && c.id == target.id)
        else {
            return false;
        };
        corpus
            .graph
            .ingredients_of(RecipeId(idx as u32))
            .iter()
            .any(|&i| corpus.ingredient_names[i.0 as usize].trim().to_lowercase() == key)
    }
}
