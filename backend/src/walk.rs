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

use std::collections::{BTreeSet, HashMap};

use axum::{
    extract::{Query, State},
    Extension, Json,
};
use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};
use recipe_core::equipment::{capability, Capability, RequiredEquipment};
use recipe_core::meal::{course, fit, Course, MealFit};
use recipe_core::{Ingredient, Sitting};
use recipe_walk::{FixtureGraph, IngredientId, RecipeGraph, RecipeId, TabuWeighted, Walk};
use serde::{Deserialize, Serialize};

use crate::auth::CurrentUser;
use crate::session::MealType;
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
    /// The pick session this walk feeds (#80, #82, #184, #202). Present, the session's
    /// bounds — its time cap, what its kitchen can make, the meal it is for, and what
    /// the caller has already answered in it — bound the corpus walked; an unknown
    /// channel is refused rather than read as "unbounded". Absent, the walk is over the
    /// whole corpus, as ever. It scopes the walk to the plan's bounds — it is not an
    /// access check (the session gate already authenticated the caller, and a bound is a
    /// filter, not a secret).
    ///
    /// It is the only half of the deal's key the client supplies. The other half — who
    /// is asking — comes from the session (#202), so no query string can deal a client
    /// somebody else's remainder.
    channel: Option<String>,
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
    /// The recipe's estimated total time (#79/#84): the critical path over its step
    /// DAG, so the swiper can weigh "do I have time for this" while voting rather
    /// than after picking. `None` is **unknown** — the step worker has not read this
    /// recipe, or it read no timed step — never "instant"; the client shows nothing
    /// for it. A `Some` is a **lower bound**: untimed steps ("until golden")
    /// contribute nothing, so the real cook takes at least this long, and the client
    /// renders it as an at-least, never an exact time — unless [`Self::fully_timed`]
    /// says otherwise.
    pub total_seconds: Option<i64>,
    /// Whether every step of this recipe carries a duration (#158/#84), so whether
    /// `total_seconds` is a complete estimate or only a floor.
    ///
    /// It rides on the card because the card is where the number is rendered and the
    /// client cannot work it out: it holds no steps here, and it could not run
    /// `recipe-core` over them if it did (no WASM). `true` → the error is ordinary
    /// estimation noise in either direction, so the mark is `~`; `false` → at least
    /// one step counted as 0, so the total can only be too low and the mark is `+`.
    /// Both are honest, and which is honest *now* is a property of the rows, not of
    /// the deploy — so the badge self-corrects as the corpus is re-read.
    pub fully_timed: bool,
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

/// What a walk is bounded to, already resolved against the database. [`Default`] is
/// unbounded — the whole corpus, which is what a walk with no channel gets.
#[derive(Debug, Clone, Default)]
struct Bounds {
    /// The pick session's time cap in seconds (#80); `None` = "Any".
    max_total_seconds: Option<i64>,
    /// The equipment the plan's kitchen is recorded as holding (#82). Already
    /// normalised (#81), so matching it is containment, never a fuzzy compare.
    ///
    /// `None` means there is nothing to match against — no kitchen, or a kitchen whose
    /// equipment nobody has recorded — and the walk is unlimited. Never `Some` of an
    /// empty set: the two would behave in opposite ways (an empty set excludes
    /// everything), and only one of them is ever true (see [`resolve_bounds`]).
    owned_equipment: Option<BTreeSet<String>>,
    /// The meal this walk is dealing (#114/#184). `Some` whenever a channel names a
    /// plan — the pick runs one round and that round is the meal — so the deck is kept
    /// clear of dishes the corpus states are accompaniments. `None` is a walk with no
    /// plan behind it, which is not a meal round and sees the whole corpus.
    meal_type: Option<MealType>,
    /// Whose round this is (#202), so it can be *continued* rather than restarted.
    /// `Some` on exactly the same condition as [`Self::meal_type`] — a channel names a
    /// plan and the session names the caller — and `None` for the plan-less walk, which
    /// is nobody's round and is untouched.
    answered_by: Option<Answered>,
}

/// The caller of a meal round, as the deal has to know them (#202): a plan and a person,
/// which is exactly the key `votes` is written under.
///
/// One field rather than two `Option`s on [`Bounds`], because the two are only ever
/// meaningful together: a channel with no voter would exclude nothing and a voter with
/// no channel would exclude across every plan they have ever swiped. Neither is a state
/// this can be put in.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Answered {
    /// The plan whose votes count. A vote is a call *in this plan*, so answering a
    /// recipe in last week's dinner cannot narrow tonight's.
    channel: String,
    /// The telegram id of the person asking, from the session (#25) — never from the
    /// query string, which would let a client deal itself somebody else's remainder.
    voter: String,
}

/// Would a plan for `meal` deal this dish **as the meal itself** (#184, #191)?
///
/// Two questions, in order, because they are answered by two different kinds of thing.
///
/// # 1. Is it the meal at all, or something that comes with one? (#184, stated)
///
/// The pick's one round is the meal round, and #114's vocabulary already rules that a
/// dessert or a side is a thing that comes *with* a meal rather than the meal a plan
/// is for. So a dish whose source states it is one — [`Course::Accompaniment`] — is not
/// in the deck a host asked for when they answered "which meal". That is a claim the
/// corpus really made (`Dessert` 166, `Side` 84, `Starter` 14 of 790), and it is the same
/// for all four meal words.
///
/// A `category` that names a protein or a style says nothing at all, and a stated
/// `Breakfast` is still not acted on **here** — deciding a breakfast dish is not a dinner
/// is a judgement, and judgements now belong to the reading below rather than to a word a
/// source filed the dish under. Both stay, which is [`Capability::Unread`]'s ruling (#82).
///
/// # 2. Is this dish eaten at this sitting? (#191, read)
///
/// This is the question #184 could not answer and this filter exists for. The corpus
/// carries no `Lunch`, `Dinner` or `Snack` category at all, so nothing it *states* can
/// tell the four meal words apart; [`recipe_core::meal::fit`] reads the meal-time
/// reading instead — a set of sittings per recipe, produced off the service — and a dish
/// whose set does not contain this meal is out of the round. **A roast finally stays out
/// of a breakfast.**
///
/// [`MealFit::Unread`] is **excluded**, by ruling (#192/#193): a meal round serves only
/// what explicitly matches the filter, and a dish nobody has read does not explicitly
/// match anything. Missing data is a scraper/enrichment gap — the fix is reading the
/// corpus, never loosening the filter. This is the same treatment the kitchen bound
/// below gives an unread equipment reading, for the same written reason: containment is
/// a proof, and admitting unread recipes beside it would mix a proof with a guess.
///
/// The cost is stated rather than hidden: until the `enrich-meal-times` worker has read
/// a recipe, no meal round deals it, and on the day this lands that is the whole corpus
/// — every meal round is empty until the worker runs. Running the worker is the act
/// that delivers the feature; ingest adding new recipes keeps them out of every round
/// until they are read, which is exactly the explicit-only behaviour asked for.
fn deals_as_the_meal(meal: Sitting, category: Option<&str>, sittings: &[Sitting]) -> bool {
    if course(category) == Course::Accompaniment {
        // Stated to accompany a meal, so never the meal — whichever meal is planned, and
        // whatever its sittings say. A trifle is eaten after dinner; that does not make
        // it the dinner, and #147's rounds are where it gets dealt.
        return false;
    }
    match fit(sittings, meal) {
        MealFit::Suits => true,
        MealFit::Wrong => false,
        MealFit::Unread => false,
    }
}

/// Load the whole normalized corpus into a [`Corpus`]. One query; the ingredients
/// column is JSON, parsed here into names (measures are irrelevant to the graph).
///
/// # The time cap (#80)
///
/// Recipes whose `total_seconds` estimate exceeds the cap never enter the graph, so a
/// capped walk cannot surface them. The comparison is inclusive — a 30-minute recipe
/// fits a 30-minute cap.
///
/// **A `NULL` estimate stays in.** That is a deliberate call, not an accident of SQL
/// three-valued logic:
///
/// - The estimate is a **lower bound** even when present (untimed steps add nothing,
///   #79) — so `total_seconds <= cap` never *proves* a recipe fits. Excluding
///   "unknown" while admitting "at least this long" would hold the two to different
///   standards for the same uncertainty.
/// - Enrichment is an addition, never a gate — a recipe must not vanish from the product
///   because a worker has not read it yet. Measured against production while #163 was
///   being landed, 77 of 790 recipes carry no estimate; the number shrinks as the worker
///   reads, and the ruling does not depend on its size, because a newly ingested recipe
///   always sits unread for a while.
///
/// Now that every plan is *born* capped (#163) this is the common path rather than the
/// exception, which is the same measurement seen from the other side: a 1800-second cap
/// leaves 390 of 790 recipes — 313 that fit it on the estimate we have, plus those 77 we
/// cannot time at all.
///
/// # The kitchen limit (#82)
///
/// A meal planned in a kitchen is cooked in that kitchen, so this is **not an option
/// anyone turns on**: whenever there is equipment to match against, a recipe enters the
/// graph only if its equipment reading (#81) is a subset of what the kitchen holds —
/// [`Capability::CanMake`]. Nobody should have to ask not to be shown a recipe needing
/// a blender they do not own.
///
/// **A recipe with no reading is left out**, which is the opposite call to the cap
/// above, and the difference is what the two filters can *prove*:
///
/// - A time estimate is a lower bound even when we have it, so the cap is choosing
///   between two flavours of uncertainty and has no reason to be stricter with one.
///   An equipment reading is **complete** — it names everything, prep tools included
///   (#81) — so containment is a proof, and admitting unread recipes beside it would
///   mix a proof with a guess.
/// - #81 already ruled on the empty list: it is *refused* on the way in, precisely so
///   that a kitchen owning no knife cannot appear able to cook everything. Reading `[]`
///   as makeable here would reinstate the exact failure that ruling prevents.
/// - The premise that made #80 include unknowns does not hold here: measured against
///   production, **790/790 recipes carry an equipment reading** in
///   `equipment_structures`, so excluding unread ones costs nothing today.
///
/// That last point is why this is an accommodation and not a semantics: a missing
/// reading is a gap in our extraction, not a property of the recipe (#158) — every
/// recipe needs *something*. Newly ingested recipes sit unread until the worker reaches
/// them, and while they do, a can-make plan will not deal them. If that window ever
/// grows big enough to matter, the fix is reading them, not loosening this.
///
/// The column read here is the *derived* one, which trails the readings by a derive:
/// `recipes.equipment` holds a reading only once one has run since it landed. It held
/// nothing at all until this branch fixed [`crate::recipes::upsert`], so a deployment
/// that has not re-derived since reads the whole corpus as unread, and a can-make plan
/// deals nothing. Ingest runs derive on its schedule, so that heals itself — but it is
/// worth knowing while it does.
///
/// The cap filters in SQL and this filters in Rust, which is not an inconsistency to
/// tidy away: a cap is a scalar comparison SQL does natively, while subset containment
/// over a JSON array — plus the empty-means-unread ruling — is a judgement that belongs
/// in one tested place ([`recipe_core::equipment::capability`]), shared with #83, rather
/// than re-encoded in a `json_each` subquery that could drift from it.
///
/// # The meal (#184, #191)
///
/// A plan is *for* a meal, and until #184 landed that choice reached the heading and
/// nothing else — a breakfast plan was dealt the same 790 recipes a dinner plan was.
/// A walk behind a channel is now a **meal round**, and two things keep it to the meal:
///
/// - a dish the corpus **states** is an accompaniment (`Dessert`, `Side`, `Starter` —
///   264 of 790) is not in it, whichever meal was asked for (#184/#114);
/// - a dish the meal-time **reading** says is eaten at other sittings is not in it
///   either (#191) — the first thing in the corpus that can tell a breakfast from a
///   dinner, since no category the corpus carries ever could.
///
/// **An unread recipe is still dealt**, which is [`Capability::Unread`]'s ruling (#82)
/// and, on the day this lands, the difference between a working deck and an empty one:
/// nothing has been read yet, because the reading is produced off the service by a
/// worker somebody runs (#59). The consequence, stated plainly: **until the corpus is
/// read a dinner plan still deals dishes nobody has said are dinners** — the same deck
/// #184 left, minus whatever has been read since. It tightens per recipe, with no flag to
/// switch. See [`recipe_core::meal::fit`].
///
/// This filters in Rust for the same reason the kitchen bound does: reading one flat
/// field for two different kinds of claim, ruling on what silence means, and ruling on
/// what an absent reading means are judgements that belong in one tested place
/// ([`recipe_core::meal`]) — which #147's per-addition rounds will read from the opposite
/// direction — rather than an `IN` list and a `json_each` subquery inlined here that
/// could drift from them.
///
/// # What you have already answered (#202)
///
/// A plan runs for days: the roster, every vote and the shopping state are all durable,
/// so people drop out and come back. The deal was not — it re-seeded from OS entropy and
/// consulted nothing about the caller, so a member returning on day four was dealt the
/// cards they answered on day one. Re-swiping overwrites (`record_vote`) so nothing was
/// *corrupted*; what was impossible was **continuing** a long plan, because every return
/// started it over.
///
/// So a meal round excludes the `(source, id)`s this caller has already voted on **in
/// this channel**. It changes what is *dealt*, never what is *writable* — the vote upsert
/// is untouched, and a change-your-mind surface would be its own work.
///
/// **This one is in SQL, beside the cap, and the two bounds above are not** — which is
/// the same distinction those two already draw, applied to a third thing:
///
/// - The kitchen and meal bounds read a **JSON reading** and rule on what an absent one
///   means. That is a judgement, it is shared with other callers (#83, #147), and it
///   belongs in one tested place in `recipe-core` rather than re-encoded in a
///   `json_each` subquery that could drift from it.
/// - This is not a judgement about a recipe at all. It is set membership over rows the
///   database already holds and already indexes by exactly this key — `votes`' primary
///   key is `(channel_id, source, id, voter_id)`, so the `NOT EXISTS` is a covered
///   lookup. Answering it in Rust would mean a second query per walk to load every vote
///   in the plan and rebuild that index in memory, on the pick page's hot path. The cap
///   sits in SQL for the same reason stated the other way round: it is what SQL does
///   natively.
///
/// There is no ruling here about silence, either: a recipe you have not voted on is a
/// recipe you have not voted on. Absence of a row is the fact, not a gap in a reading —
/// which is precisely why this needs none of the care #82 and #192 needed.
///
/// **A `no` is an answer.** The exclusion is on the *row*, not on `vote = 1`: passing on
/// a recipe is deciding about it, and re-dealing it would be the same "starting over" the
/// issue is about.
///
/// The predicate is `?2 IS NULL OR NOT EXISTS (…)`, so a plan-less walk — which carries
/// no channel and therefore no voter — is untouched, exactly like the meal bound above.
/// Note the cap's clause is parenthesised: `AND` binds tighter than `OR`, so without the
/// brackets this would have attached to `total_seconds <= ?1` alone and left every
/// un-estimated recipe dealt forever.
///
/// If a plan is ever **decided**, the decided state wins and this defers to it — the deal
/// is not what such a plan is showing (#202).
async fn load_corpus(conn: &libsql::Connection, bounds: &Bounds) -> anyhow::Result<Corpus> {
    // Both or neither: `Answered` cannot hold one without the other, and the SQL keys off
    // `?2` alone, so they cannot drift apart into a half-applied exclusion.
    let (channel, voter) = match &bounds.answered_by {
        Some(a) => (Some(a.channel.as_str()), Some(a.voter.as_str())),
        None => (None, None),
    };
    let mut rows = conn
        .query(
            "SELECT source, id, title, image, category, area, total_seconds, fully_timed, ingredients, equipment, sittings
             FROM recipes
             WHERE (?1 IS NULL OR total_seconds IS NULL OR total_seconds <= ?1)
               AND (?2 IS NULL OR NOT EXISTS (
                     SELECT 1 FROM votes
                      WHERE votes.channel_id = ?2
                        AND votes.voter_id = ?3
                        AND votes.source = recipes.source
                        AND votes.id = recipes.id))",
            libsql::params![bounds.max_total_seconds, channel, voter],
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
            // The same column the cap filters on (#80) now also rides out to the
            // card (#84): the walk already holds it, so showing the estimate costs
            // one more column, not a second read.
            total_seconds: row.get::<Option<i64>>(6)?,
            // NOT NULL DEFAULT 0, so this is a plain read with no absent case: a row
            // the step worker has not reached is `0`, which is the truth about it.
            fully_timed: row.get::<i64>(7)? != 0,
        };
        // The meal bound (#184, #191). Before the ingredients are parsed, because a dish
        // that is not in this round has no reason to be read at all.
        if let Some(meal) = bounds.meal_type {
            // `sittings` is our own serialization, NOT NULL DEFAULT '[]' — so the same
            // split as the ingredients and equipment below: a column-read error is
            // structural and propagates, an unparseable value degrades this one recipe.
            // It degrades to *unread*, which is the reading that restricts nothing, so a
            // corrupt row stays in the deck rather than silently vanishing from a plan.
            let json = row.get::<String>(10)?;
            let sittings: Vec<Sitting> = serde_json::from_str(&json).unwrap_or_else(|e| {
                tracing::warn!(
                    "recipe {}/{} has unparseable sittings JSON, treating as unread: {e}",
                    card.source,
                    card.id
                );
                Vec::new()
            });
            if !deals_as_the_meal(meal, card.category.as_deref(), &sittings) {
                continue;
            }
        }
        // The ingredients column is our own serialization — NOT NULL DEFAULT '[]',
        // written only by ingest — so the two ways to fail here are not the same.
        // A column-read error is *structural*: the column is gone or the wrong
        // type, which is schema drift affecting every row, so it propagates and
        // fails the request loudly, the way a wrong DATABASE_URL does (see db.rs).
        // A JSON *parse* error is per-row: one corrupt value must not 500 a walk
        // that works over the other recipes, so that recipe degrades to an
        // edgeless node — but it is warned, not dropped silently, so corruption is
        // still visible.
        let json = row.get::<String>(8)?;
        let ingredients: Vec<Ingredient> = serde_json::from_str(&json).unwrap_or_else(|e| {
            tracing::warn!(
                "recipe {}/{} has unparseable ingredients JSON, treating as none: {e}",
                card.source,
                card.id
            );
            Vec::new()
        });
        // The kitchen bound (#82). Same split as the ingredients above: a structural
        // read error propagates, an unparseable value degrades this one recipe. It
        // degrades to *unread* — an empty reading, which is exactly the reading we
        // cannot act on — so a corrupt row is left out of a can-make walk rather than
        // waved through as makeable.
        if let Some(owned) = &bounds.owned_equipment {
            let json = row.get::<String>(9)?;
            let required: Vec<RequiredEquipment> =
                serde_json::from_str(&json).unwrap_or_else(|e| {
                    tracing::warn!(
                        "recipe {}/{} has unparseable equipment JSON, treating as unread: {e}",
                        card.source,
                        card.id
                    );
                    Vec::new()
                });
            if capability(&required, owned) != Capability::CanMake {
                continue;
            }
        }
        let names = ingredients.into_iter().map(|i| i.name).collect();
        out.push((card, names));
    }

    Ok(Corpus::build(out))
}

/// Resolve what a walk is bounded to. No channel is unbounded; a channel that names no
/// session is refused rather than silently walked over the whole corpus, which would
/// hand a mistyped channel the deck the plan asked not to have (#80).
///
/// The kitchen's inventory is read here, per walk, rather than frozen onto the plan:
/// what a kitchen owns is a fact about the world, not a setting on this plan, so
/// remembering the stand mixer widens the deck immediately instead of needing a new
/// plan (#82).
///
/// **A kitchen with nothing recorded limits nothing.** That is the same ruling #81
/// already made about readings — an empty equipment reading was *refused*, because
/// "needs nothing" is never true of a recipe; a salad still needs bowls and knives —
/// applied to the other side of the same comparison. A kitchen with no equipment
/// recorded is a kitchen **we have not recorded**, not a kitchen with no tools, so
/// matching against it would prove nothing while excluding everything. Measured: the
/// only kitchen in production holds zero items, so reading zero as a claim would deal
/// every real user an empty pick.
///
/// It is a **gap, not a preference** (#158). The honest state is "we do not know what
/// this kitchen has", and the answer to a gap is filling it, not narrowing the product
/// against it — so the deck stays whole and the lobby says why, rather than leaving a
/// wider deck unexplained.
///
/// `voter` is the caller's telegram id, taken from the session this route is already
/// gated on (#25). It is what makes a meal round *this person's* deal (#202) — the walk
/// was channelled and session-gated all along, so both halves of the key were in hand
/// already and nothing new is asked of the client.
async fn resolve_bounds(
    state: &AppState,
    channel: Option<&str>,
    voter: &str,
) -> Result<Bounds, AppError> {
    let Some(channel) = channel else {
        return Ok(Bounds::default());
    };
    let plan = state
        .with_db(move |conn| async move { crate::session::plan_bounds(&conn, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;

    let owned_equipment = match plan.kitchen_id.as_deref() {
        Some(kitchen_id) => {
            let owned: BTreeSet<String> = state
                .with_db(move |conn| async move {
                    crate::kitchens::equipment_of(&conn, kitchen_id).await
                })
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?
                .into_iter()
                .collect();
            // Nothing recorded collapses to `None` — unlimited — never `Some(empty)`,
            // which would exclude the whole corpus. See this function's doc.
            (!owned.is_empty()).then_some(owned)
        }
        None => None,
    };
    Ok(Bounds {
        max_total_seconds: plan.max_total_seconds,
        owned_equipment,
        // A channel names a plan, and the pick's one round deals that plan's meal, so a
        // channelled walk is always a meal round (#184). `None` — the unbounded,
        // channel-less walk — is the only walk that is not one.
        meal_type: Some(plan.meal_type),
        // …and the same walk is somebody's round, so it continues where they left off
        // rather than restarting (#202). Set on the same condition and in the same
        // breath as the meal, because it is the same fact: a channelled walk is a meal
        // round, and a meal round is dealt to a person.
        answered_by: Some(Answered {
            channel: channel.to_owned(),
            voter: voter.to_owned(),
        }),
    })
}

/// `GET /api/walk?len=<n>&channel=<pick>` — a fresh variety-first walk over the
/// corpus, bounded to the pick session's time cap (#80), to what its kitchen can make
/// (#82), and to the meal it is for (#184) whenever a channel is named.
///
/// Session-gated like every person-facing route (#25). Each call re-seeds from OS
/// entropy, so the same corpus yields a different journey every time — freshness is
/// the whole point (#47).
///
/// The session is now *read* as well as required: a meal round skips what this caller
/// has already answered in this plan (#202), so a plan that runs for days is continued
/// rather than restarted. Freshness and continuity are the same wish — the point of a
/// re-seed was never to deal the same card twice.
pub async fn walk(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Query(params): Query<WalkParams>,
) -> Result<Json<WalkResponse>, AppError> {
    let len = params.len.unwrap_or(DEFAULT_LEN).clamp(1, MAX_LEN);
    // The bounds are read fresh from the session on every walk, so the walk always
    // enforces what the plan currently says — the client passes only the channel,
    // never a bound itself, which keeps them server-authoritative (#80). Who is asking
    // comes from the session for the same reason: a voter id in the query string would
    // let a client deal itself the cards somebody else has left (#202).
    let bounds = resolve_bounds(&state, params.channel.as_deref(), &user.telegram_user_id).await?;
    let bounds = &bounds;
    let corpus = state
        .with_db(move |conn| async move { load_corpus(&conn, bounds).await })
        .await
        .map_err(|e| AppError::Internal(format!("could not load the corpus: {e:#}")))?;
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
            total_seconds: None,
            fully_timed: false,
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

    /// A migrated in-memory corpus with one recipe per estimate shape (#80):
    /// well under, exactly on, and well over a 30-minute cap, plus one with no
    /// estimate at all (`NULL` — not step-read yet).
    async fn seeded_conn() -> libsql::Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        for (id, secs) in [
            ("quick", Some(900i64)),
            ("exact", Some(1800i64)),
            ("slow", Some(7200i64)),
            ("unknown", None),
        ] {
            conn.execute(
                "INSERT INTO recipes (source, id, title, total_seconds)
                 VALUES ('test', ?1, ?1, ?2)",
                libsql::params![id, secs],
            )
            .await
            .unwrap();
        }
        conn
    }

    fn ids(corpus: &Corpus) -> Vec<&str> {
        let mut v: Vec<&str> = corpus.cards.iter().map(|c| c.id.as_str()).collect();
        v.sort();
        v
    }

    /// Bounded by time alone (#80).
    fn capped(seconds: i64) -> Bounds {
        Bounds {
            max_total_seconds: Some(seconds),
            ..Bounds::default()
        }
    }

    /// Bounded to a kitchen holding exactly `items` (#82).
    fn owning(items: &[&str]) -> Bounds {
        Bounds {
            owned_equipment: Some(items.iter().map(|i| (*i).to_owned()).collect()),
            ..Bounds::default()
        }
    }

    /// Bounded to a plan's meal round (#184) and nothing else.
    fn planning(meal: MealType) -> Bounds {
        Bounds {
            meal_type: Some(meal),
            ..Bounds::default()
        }
    }

    /// No cap — the whole corpus, exactly as before the cap existed.
    #[tokio::test]
    async fn an_uncapped_load_takes_the_whole_corpus() {
        let conn = seeded_conn().await;
        let corpus = load_corpus(&conn, &Bounds::default()).await.unwrap();
        assert_eq!(ids(&corpus), vec!["exact", "quick", "slow", "unknown"]);
    }

    /// A capped load drops what is estimated over the cap and keeps the rest:
    /// under-cap fits, exactly-on-cap fits (the bound is inclusive — a 30-minute
    /// recipe fits a 30-minute cap), and an unknown estimate stays in (see
    /// [`load_corpus`] for the policy).
    #[tokio::test]
    async fn a_cap_excludes_recipes_estimated_over_it() {
        let conn = seeded_conn().await;
        let corpus = load_corpus(&conn, &capped(1800)).await.unwrap();
        assert_eq!(ids(&corpus), vec!["exact", "quick", "unknown"]);
    }

    /// The `NULL` policy under pressure: a cap below every timed estimate leaves
    /// only the un-estimated recipe — included, not excluded, because the estimate
    /// is a lower bound even when present and enrichment is never a gate. The
    /// corpus narrows; it does not vanish.
    #[tokio::test]
    async fn an_unknown_estimate_stays_in_a_capped_walk() {
        let conn = seeded_conn().await;
        let corpus = load_corpus(&conn, &capped(600)).await.unwrap();
        assert_eq!(ids(&corpus), vec!["unknown"]);
    }

    /// The estimate rides out **on the card** (#84), not only in the cap's `WHERE`
    /// clause (#80): the swiper weighs "do I have time for this" while voting, not
    /// after picking. `NULL` arrives as `None` — unknown, which the client renders
    /// as nothing rather than as "0 min".
    ///
    /// This also pins the column order. The `SELECT` names its columns and the reads
    /// are positional, so a column inserted in the middle without moving the indexes
    /// silently reads the wrong value — the shape of the #109 outage.
    #[tokio::test]
    async fn a_loaded_card_carries_its_time_estimate() {
        let conn = seeded_conn().await;
        let corpus = load_corpus(&conn, &Bounds::default()).await.unwrap();
        let estimate = |id: &str| {
            corpus
                .cards
                .iter()
                .find(|c| c.id == id)
                .expect("seeded recipe is in the corpus")
                .total_seconds
        };
        assert_eq!(estimate("quick"), Some(900));
        assert_eq!(estimate("exact"), Some(1800));
        assert_eq!(estimate("slow"), Some(7200));
        assert_eq!(estimate("unknown"), None);
    }

    /// A corpus shaped for the kitchen bound (#82): one recipe per relationship a
    /// kitchen can have with a reading — needs nothing you lack, needs one thing you
    /// lack, needs several, and one with no reading at all.
    async fn equipment_conn() -> libsql::Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        for (id, equipment) in [
            ("simple", r#"[{"item":"knife"},{"item":"bowl"}]"#),
            (
                "exact",
                r#"[{"item":"knife"},{"item":"bowl"},{"item":"wok"}]"#,
            ),
            ("blender", r#"[{"item":"knife"},{"item":"blender"}]"#),
            ("fancy", r#"[{"item":"sous vide"},{"item":"blowtorch"}]"#),
            ("unread", "[]"),
        ] {
            conn.execute(
                "INSERT INTO recipes (source, id, title, equipment)
                 VALUES ('test', ?1, ?1, ?2)",
                libsql::params![id, equipment],
            )
            .await
            .unwrap();
        }
        conn
    }

    /// Unbounded — the whole corpus, unread recipes and all, exactly as before the
    /// bound existed. Nothing about #82 narrows a plan that did not ask for it.
    #[tokio::test]
    async fn an_unbounded_load_keeps_unread_recipes() {
        let conn = equipment_conn().await;
        let corpus = load_corpus(&conn, &Bounds::default()).await.unwrap();
        assert_eq!(
            ids(&corpus),
            vec!["blender", "exact", "fancy", "simple", "unread"]
        );
    }

    /// The bound is subset containment: everything the reading names must be owned.
    /// Owning exactly the reading is enough (`exact`), owning more than it asks for is
    /// fine (`simple`), and one missing item is enough to drop it (`blender`).
    #[tokio::test]
    async fn a_kitchen_bound_keeps_only_what_it_can_make() {
        let conn = equipment_conn().await;
        let corpus = load_corpus(&conn, &owning(&["knife", "bowl", "wok"]))
            .await
            .unwrap();
        assert_eq!(ids(&corpus), vec!["exact", "simple"]);
    }

    /// **The unknown-reading policy** (#82), and the one place it differs from the
    /// time cap: a recipe with no equipment reading is *left out* of a can-make walk.
    ///
    /// A reading is complete when we have one — prep tools included (#81) — so
    /// containment is a proof; `[]` is not a weaker reading but no reading, refused as
    /// one on the way in precisely so "needs nothing" can never be mistaken for
    /// "anyone can cook it". Admitting it here would put a guess in a deck whose one
    /// promise is that you can make what is in it.
    #[tokio::test]
    async fn an_unread_recipe_is_left_out_of_a_kitchen_bound_walk() {
        let conn = equipment_conn().await;
        for owned in [
            owning(&["knife", "bowl", "wok"]),
            owning(&["knife"]),
            owning(&[]),
        ] {
            let corpus = load_corpus(&conn, &owned).await.unwrap();
            assert!(
                !ids(&corpus).contains(&"unread"),
                "an unread recipe is never proven makeable: {:?}",
                ids(&corpus)
            );
        }
    }

    /// A kitchen with nothing recorded is represented as *nothing to match against*
    /// (`None`) and never as an empty set, so it limits nothing — `resolve_bounds`
    /// collapses the one to the other. Pinned here because the two are one keystroke
    /// apart and behave in opposite ways: an empty set excludes the entire corpus.
    #[tokio::test]
    async fn nothing_recorded_is_unlimited_not_empty() {
        let conn = equipment_conn().await;
        let unlimited = load_corpus(&conn, &Bounds::default()).await.unwrap();
        assert_eq!(
            unlimited.cards.len(),
            5,
            "the whole corpus, unread included"
        );

        let empty_set = load_corpus(&conn, &owning(&[])).await.unwrap();
        assert!(
            ids(&empty_set).is_empty(),
            "which is exactly why an unrecorded kitchen must not arrive as one"
        );
    }

    /// The two bounds compose: over-cap recipes go by time, un-makeable ones by
    /// equipment, and only what survives both is dealt.
    #[tokio::test]
    async fn the_time_cap_and_the_kitchen_bound_compose() {
        let conn = equipment_conn().await;
        conn.execute(
            "UPDATE recipes SET total_seconds = 7200 WHERE id = 'simple'",
            (),
        )
        .await
        .unwrap();
        let bounds = Bounds {
            max_total_seconds: Some(1800),
            owned_equipment: Some(
                ["knife", "bowl", "wok"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            ),
            ..Bounds::default()
        };
        let corpus = load_corpus(&conn, &bounds).await.unwrap();
        assert_eq!(
            ids(&corpus),
            vec!["exact"],
            "`simple` is makeable but too slow; the others are quick but not makeable"
        );
    }

    /// A corrupt reading degrades to *unread*, so it is left out of a can-make walk
    /// rather than waved through — the safe direction for a deck that promises
    /// makeability. Unbounded, the recipe is untouched: the bound is what reads the
    /// column, so nothing else can be broken by it.
    #[tokio::test]
    async fn an_unparseable_reading_is_treated_as_unread() {
        let conn = equipment_conn().await;
        conn.execute(
            "UPDATE recipes SET equipment = '{oops' WHERE id = 'simple'",
            (),
        )
        .await
        .unwrap();

        let bounded = load_corpus(&conn, &owning(&["knife", "bowl", "wok"]))
            .await
            .unwrap();
        assert_eq!(ids(&bounded), vec!["exact"]);

        let all = load_corpus(&conn, &Bounds::default()).await.unwrap();
        assert!(ids(&all).contains(&"simple"));
    }

    // ---- the meal bound (#184) ---------------------------------------------

    /// A corpus shaped for the meal bound: one recipe per thing a `category` can state.
    /// The three accompaniment words the corpus uses, the one sitting it states, a
    /// category that names a protein and says nothing about the meal, and the two ways
    /// to say nothing at all.
    async fn meal_conn() -> libsql::Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        for (id, category) in [
            ("trifle", Some("Dessert")),
            ("chips", Some("Side")),
            ("soup", Some("Starter")),
            // The source's own spelling reaches the column unnormalised, so the rule
            // has to fold case here rather than trust the corpus to be tidy.
            ("brownie", Some("dessert")),
            ("pancakes", Some("Breakfast")),
            ("stew", Some("Beef")),
            ("blank", Some("   ")),
            ("uncategorised", None),
        ] {
            conn.execute(
                "INSERT INTO recipes (source, id, title, category)
                 VALUES ('test', ?1, ?1, ?2)",
                libsql::params![id, category],
            )
            .await
            .unwrap();
        }
        conn
    }

    /// **The fix** (#184): a plan's round deals the meal, so a dish the corpus *states*
    /// accompanies one is not in the deck. `Dessert`, `Side` and `Starter` are the
    /// three words it uses, whatever case they arrive in.
    #[tokio::test]
    async fn a_meal_round_refuses_a_stated_accompaniment() {
        let conn = meal_conn().await;
        let corpus = load_corpus(&conn, &planning(MealType::Dinner))
            .await
            .unwrap();
        for refused in ["trifle", "chips", "soup", "brownie"] {
            assert!(
                !ids(&corpus).contains(&refused),
                "the corpus states {refused} accompanies a meal: {:?}",
                ids(&corpus)
            );
        }
    }

    /// **The ruling that does most of the work** (#82's `Capability::Unread`, applied
    /// to the other reading): a recipe the corpus says nothing about stays in.
    ///
    /// This is the whole difference between the filter that landed and the one that
    /// looked obvious. `Beef` is a claim about beef, not a claim that a dish is dinner
    /// — and by the same coin it is not a claim that the dish *is* anything a meal
    /// round could serve. A meal round serves only what explicitly matches the filter
    /// (#192), so a recipe the corpus says nothing about is in no meal round at all:
    /// missing data is a scraper/enrichment gap, and the fix is reading the corpus,
    /// never widening the deck to cover for it.
    #[tokio::test]
    async fn a_recipe_the_corpus_says_nothing_about_is_in_no_meal_round() {
        let conn = meal_conn().await;
        let corpus = load_corpus(&conn, &planning(MealType::Dinner))
            .await
            .unwrap();
        assert!(
            ids(&corpus).is_empty(),
            "nothing here is read, so nothing explicitly matches a dinner: {:?}",
            ids(&corpus)
        );
    }

    /// A stated `Breakfast` **category is not a sittings reading**. The filter serves
    /// explicit matches of the reading only, so even the 19 recipes the source files
    /// under `Breakfast` wait for the worker like everything else — one rule, no side
    /// door. (Whether derive should seed a sitting from a stated category is a design
    /// question for the human; until ruled, a category neither admits nor excludes a
    /// meal — it only names accompaniments.)
    #[tokio::test]
    async fn a_stated_breakfast_category_is_not_a_reading() {
        let conn = meal_conn().await;
        for meal in [
            MealType::Breakfast,
            MealType::Lunch,
            MealType::Dinner,
            MealType::Snack,
        ] {
            let corpus = load_corpus(&conn, &planning(meal)).await.unwrap();
            assert!(
                !ids(&corpus).contains(&"pancakes"),
                "unread, so no {meal:?} plan deals it — not even breakfast"
            );
        }
    }

    /// **An unread corpus deals an empty meal round**, and that is what the day this
    /// lands looks like — stated plainly rather than discovered. The requirement is
    /// explicit meals (#192); nothing unread explicitly matches anything, so the deck
    /// is empty until the `enrich-meal-times` worker has read the corpus. Running the
    /// worker is the act that delivers the feature; deploying this is not it.
    #[tokio::test]
    async fn an_unread_corpus_deals_an_empty_meal_round() {
        let conn = meal_conn().await;
        for meal in Sitting::ALL {
            let corpus = load_corpus(&conn, &planning(meal)).await.unwrap();
            assert!(
                ids(&corpus).is_empty(),
                "a {meal:?} plan over an unread corpus deals nothing: {:?}",
                ids(&corpus)
            );
        }
    }

    /// A walk with no plan behind it is not a meal round, so it still sees everything —
    /// nothing about #184 narrows a caller that named no channel.
    #[tokio::test]
    async fn a_walk_with_no_plan_still_sees_accompaniments() {
        let conn = meal_conn().await;
        let corpus = load_corpus(&conn, &Bounds::default()).await.unwrap();
        assert_eq!(
            ids(&corpus),
            vec![
                "blank",
                "brownie",
                "chips",
                "pancakes",
                "soup",
                "stew",
                "trifle",
                "uncategorised"
            ]
        );
    }

    /// The three bounds compose: over-cap recipes go by time, un-makeable ones by
    /// equipment, stated accompaniments by the meal — and only what survives all three
    /// is dealt.
    #[tokio::test]
    async fn the_meal_bound_composes_with_the_time_cap_and_the_kitchen() {
        let conn = meal_conn().await;
        // Everything a dinner: the bounds under test are time, kitchen and
        // accompaniment, so the sittings reading is satisfied uniformly first.
        for id in ["blank", "pancakes", "stew", "uncategorised"] {
            read_as(&conn, id, &[Sitting::Dinner]).await;
        }
        conn.execute(
            "UPDATE recipes SET total_seconds = 900, equipment = '[{\"item\":\"knife\"}]'",
            (),
        )
        .await
        .unwrap();
        // `stew` is slow, `uncategorised` needs a tool this kitchen lacks, `trifle` is
        // a stated dessert. `pancakes` and `blank` survive all three.
        conn.execute(
            "UPDATE recipes SET total_seconds = 7200 WHERE id = 'stew'",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "UPDATE recipes SET equipment = '[{\"item\":\"blowtorch\"}]' WHERE id = 'uncategorised'",
            (),
        )
        .await
        .unwrap();
        let bounds = Bounds {
            max_total_seconds: Some(1800),
            owned_equipment: Some([String::from("knife")].into_iter().collect()),
            meal_type: Some(MealType::Dinner),
            // Nobody has answered anything, so #202 narrows nothing here — the three
            // bounds under test are time, kitchen and meal.
            answered_by: None,
        };
        let corpus = load_corpus(&conn, &bounds).await.unwrap();
        assert_eq!(ids(&corpus), vec!["blank", "pancakes"]);
    }

    // ---- the meal-time reading (#191) --------------------------------------

    /// Store a reading against one of [`meal_conn`]'s recipes — the row `derive` would
    /// have written from `meal_time_structures`.
    async fn read_as(conn: &libsql::Connection, id: &str, sittings: &[Sitting]) {
        conn.execute(
            "UPDATE recipes SET sittings = ?2 WHERE id = ?1",
            libsql::params![id, serde_json::to_string(sittings).unwrap()],
        )
        .await
        .unwrap();
    }

    /// **The whole point of #191.** A dish read as a dinner is not dealt to a breakfast,
    /// and one read as breakfast-or-snack is not dealt to a dinner. Nothing the corpus
    /// *states* could do this — there is no `Dinner` category — so this is the first
    /// thing in the app that tells the four meal words apart.
    #[tokio::test]
    async fn a_read_dish_is_dealt_only_to_the_sittings_it_names() {
        let conn = meal_conn().await;
        read_as(&conn, "stew", &[Sitting::Dinner]).await;
        read_as(&conn, "pancakes", &[Sitting::Breakfast, Sitting::Snack]).await;

        let breakfast = load_corpus(&conn, &planning(MealType::Breakfast))
            .await
            .unwrap();
        assert!(
            !ids(&breakfast).contains(&"stew"),
            "a roast finally stays out of a breakfast: {:?}",
            ids(&breakfast)
        );
        assert!(ids(&breakfast).contains(&"pancakes"));

        let dinner = load_corpus(&conn, &planning(MealType::Dinner))
            .await
            .unwrap();
        assert!(ids(&dinner).contains(&"stew"));
        assert!(
            !ids(&dinner).contains(&"pancakes"),
            "and a breakfast dish read as one stays out of a dinner: {:?}",
            ids(&dinner)
        );

        // A dish read for several sittings is dealt to each of them, which is why the
        // reading is a set: one label would have had to pick.
        let snack = load_corpus(&conn, &planning(MealType::Snack))
            .await
            .unwrap();
        assert!(ids(&snack).contains(&"pancakes"));
    }

    /// **The ruling, at the surface (#192).** Only what is explicitly read for this
    /// meal is dealt. An unread dish sits in no meal round — not "every round until we
    /// know better" — and each recipe joins the decks the moment its reading lands.
    /// Ingest works the same way: a freshly scraped recipe is in no round until read,
    /// which is exactly the explicit-only behaviour asked for.
    #[tokio::test]
    async fn an_unread_dish_is_dealt_to_no_meal_round() {
        let conn = meal_conn().await;
        read_as(&conn, "stew", &[Sitting::Dinner]).await;

        for meal in Sitting::ALL {
            let corpus = load_corpus(&conn, &planning(meal)).await.unwrap();
            for unread in ["blank", "uncategorised", "pancakes"] {
                assert!(
                    !ids(&corpus).contains(&unread),
                    "{unread} has no reading, so no {meal:?} plan deals it"
                );
            }
            assert_eq!(
                ids(&corpus).contains(&"stew"),
                meal == Sitting::Dinner,
                "the one recipe that has been read is dealt exactly where its reading says"
            );
        }
    }

    /// The inverse of [`an_unread_corpus_deals_an_empty_meal_round`]:
    /// once the corpus is read, the four decks are genuinely different. This is the
    /// requirement — a plan for a meal shows recipes that are explicitly that meal — and
    /// it is met by reading the corpus, not by deploying this.
    #[tokio::test]
    async fn a_read_corpus_finally_tells_the_four_meals_apart() {
        let conn = meal_conn().await;
        read_as(&conn, "pancakes", &[Sitting::Breakfast]).await;
        read_as(&conn, "stew", &[Sitting::Lunch, Sitting::Dinner]).await;
        read_as(&conn, "blank", &[Sitting::Snack]).await;
        read_as(&conn, "uncategorised", &[Sitting::Dinner]).await;

        for (meal, expected) in [
            (MealType::Breakfast, "pancakes"),
            (MealType::Lunch, "stew"),
            (MealType::Dinner, "stew,uncategorised"),
            (MealType::Snack, "blank"),
        ] {
            let deck = ids(&load_corpus(&conn, &planning(meal)).await.unwrap()).join(",");
            assert_eq!(deck, expected, "the {meal:?} deck");
        }
    }

    /// The two rules compose in the right order: a stated accompaniment is out of the
    /// meal round **whatever** its reading says. A trifle is eaten after dinner, and
    /// reading it as `["dinner"]` is correct — it still is not the dinner, and #147's
    /// per-addition rounds are what will deal it. Pinned so a reading can never
    /// re-admit what #114's vocabulary ruled out.
    #[tokio::test]
    async fn a_reading_cannot_put_an_accompaniment_back_in_the_meal_round() {
        let conn = meal_conn().await;
        read_as(&conn, "trifle", &[Sitting::Dinner]).await;
        read_as(&conn, "chips", &[Sitting::Lunch, Sitting::Dinner]).await;

        let corpus = load_corpus(&conn, &planning(MealType::Dinner))
            .await
            .unwrap();
        for refused in ["trifle", "chips"] {
            assert!(
                !ids(&corpus).contains(&refused),
                "the corpus states {refused} accompanies a meal: {:?}",
                ids(&corpus)
            );
        }
    }

    /// A corrupt reading degrades to **unread**, which restricts nothing, so the recipe
    /// is *unread*, never *wrong*: under #192 both are out of a meal round, but only
    /// unread is repaired by the next read — and the recipe stays in the corpus (a
    /// plan-less walk still deals it), so nothing silently vanishes from the product.
    #[tokio::test]
    async fn an_unparseable_reading_degrades_to_unread_not_to_wrong() {
        let conn = meal_conn().await;
        conn.execute(
            "UPDATE recipes SET sittings = 'not json' WHERE id = 'stew'",
            (),
        )
        .await
        .unwrap();
        // Corrupt is *unread*, and unread is in no meal round (#192) — same outcome,
        // different fact: a re-read repairs it, where `Wrong` would be a stored claim.
        for meal in Sitting::ALL {
            let corpus = load_corpus(&conn, &planning(meal)).await.unwrap();
            assert!(
                !ids(&corpus).contains(&"stew"),
                "a corrupt reading is unread, and unread is in no {meal:?} round"
            );
        }
        // It is degraded, not lost: a walk with no plan behind it still deals it.
        let all = load_corpus(&conn, &Bounds::default()).await.unwrap();
        assert!(
            ids(&all).contains(&"stew"),
            "still in the corpus: {:?}",
            ids(&all)
        );
    }

    /// A walk with no plan behind it is not a meal round at all, so a reading narrows
    /// nothing there either — the channel-less walk still sees the whole corpus.
    #[tokio::test]
    async fn a_walk_with_no_plan_ignores_the_reading() {
        let conn = meal_conn().await;
        read_as(&conn, "stew", &[Sitting::Dinner]).await;
        let corpus = load_corpus(&conn, &Bounds::default()).await.unwrap();
        assert_eq!(ids(&corpus).len(), 8, "every seeded recipe");
    }

    // ---- continuing a plan (#202) ------------------------------------------

    /// [`meal_conn`], with every dish the corpus does not state is an accompaniment read
    /// as a dinner. So a dinner round admits exactly `blank`, `pancakes`, `stew` and
    /// `uncategorised`, and the only thing left to narrow it is what the caller has
    /// already answered — which is what these tests are about.
    async fn dinner_conn() -> libsql::Connection {
        let conn = meal_conn().await;
        for id in ["blank", "pancakes", "stew", "uncategorised"] {
            read_as(&conn, id, &[Sitting::Dinner]).await;
        }
        conn
    }

    /// The row a swipe leaves behind — one per (plan, recipe, person), exactly as
    /// [`crate::session`] writes it. `yes` is the call, not whether it counts: a pass is
    /// an answer too.
    async fn answered(conn: &libsql::Connection, channel: &str, id: &str, voter: &str, yes: bool) {
        conn.execute(
            "INSERT INTO votes (channel_id, source, id, voter_id, vote)
             VALUES (?1, 'test', ?2, ?3, ?4)",
            libsql::params![channel, id, voter, yes as i64],
        )
        .await
        .unwrap();
    }

    /// A meal round dealt to a person (#202) — the shape [`resolve_bounds`] builds for
    /// every channelled walk: the plan's meal, plus who is asking and where.
    fn continuing(meal: MealType, channel: &str, voter: &str) -> Bounds {
        Bounds {
            meal_type: Some(meal),
            answered_by: Some(Answered {
                channel: channel.to_owned(),
                voter: voter.to_owned(),
            }),
            ..Bounds::default()
        }
    }

    /// **The fix** (#202): a card you have answered is not dealt to you again, so a plan
    /// that runs for days is *continued* rather than restarted every time you come back.
    #[tokio::test]
    async fn an_answered_card_is_not_dealt_to_its_voter_again() {
        let conn = dinner_conn().await;
        answered(&conn, "plan", "stew", "mel", true).await;
        let deck = load_corpus(&conn, &continuing(MealType::Dinner, "plan", "mel"))
            .await
            .unwrap();
        assert_eq!(
            ids(&deck),
            vec!["blank", "pancakes", "uncategorised"],
            "the one she has answered is gone; the rest of the round is not"
        );
    }

    /// **A pass is an answer.** The exclusion is on the vote *row*, not on `vote = 1`:
    /// saying no to a recipe is deciding about it, and re-dealing it is the same
    /// starting-over the yes case is. Pinned separately because `AND vote = 1` is one
    /// clause away and would leave every no in circulation forever.
    #[tokio::test]
    async fn a_pass_is_an_answer_too() {
        let conn = dinner_conn().await;
        answered(&conn, "plan", "stew", "mel", false).await;
        let deck = load_corpus(&conn, &continuing(MealType::Dinner, "plan", "mel"))
            .await
            .unwrap();
        assert!(
            !ids(&deck).contains(&"stew"),
            "she said no, which is an answer: {:?}",
            ids(&deck)
        );
    }

    /// **The deal is per person, and that is the whole point of keying it to the
    /// session.** Mel's answer narrows Mel's deck and nobody else's — a pick is people
    /// swiping the same corpus independently and converging, so a card kit has never
    /// seen is still kit's to answer.
    #[tokio::test]
    async fn a_card_somebody_else_answered_is_still_dealt_to_you() {
        let conn = dinner_conn().await;
        answered(&conn, "plan", "stew", "mel", true).await;

        let mel = load_corpus(&conn, &continuing(MealType::Dinner, "plan", "mel"))
            .await
            .unwrap();
        assert!(
            !ids(&mel).contains(&"stew"),
            "hers is answered: {:?}",
            ids(&mel)
        );

        let kit = load_corpus(&conn, &continuing(MealType::Dinner, "plan", "kit"))
            .await
            .unwrap();
        assert!(
            ids(&kit).contains(&"stew"),
            "kit has answered nothing, so kit is dealt everything: {:?}",
            ids(&kit)
        );
    }

    /// **A vote is a call in *this* plan.** Answering a recipe in last week's dinner must
    /// not narrow tonight's — the two are separate decisions by the same person, and the
    /// corpus is not a to-do list you tick off once.
    #[tokio::test]
    async fn an_answer_in_another_plan_does_not_narrow_this_one() {
        let conn = dinner_conn().await;
        answered(&conn, "last-week", "stew", "mel", true).await;
        let deck = load_corpus(&conn, &continuing(MealType::Dinner, "tonight", "mel"))
            .await
            .unwrap();
        assert!(
            ids(&deck).contains(&"stew"),
            "a different plan is a different decision: {:?}",
            ids(&deck)
        );
    }

    /// **The plan-less walk is not a meal round and is untouched** (#202), exactly as it
    /// is untouched by the meal bound above. It carries no channel, so there is nobody
    /// whose votes could narrow it — every recipe is still dealt however much this
    /// person has answered elsewhere.
    #[tokio::test]
    async fn a_walk_with_no_plan_deals_everything_however_much_you_have_answered() {
        let conn = dinner_conn().await;
        for id in [
            "blank",
            "brownie",
            "chips",
            "pancakes",
            "soup",
            "stew",
            "trifle",
            "uncategorised",
        ] {
            answered(&conn, "plan", id, "mel", true).await;
        }
        let all = load_corpus(&conn, &Bounds::default()).await.unwrap();
        assert_eq!(
            ids(&all),
            vec![
                "blank",
                "brownie",
                "chips",
                "pancakes",
                "soup",
                "stew",
                "trifle",
                "uncategorised"
            ],
            "no channel, no round, nothing to continue"
        );
    }

    /// **It composes with #192's filter rather than relaxing it.** Answering everything
    /// the round can deal empties the deck — the honest state, which the client says
    /// ("you've answered everything") instead of hunting forever. The stated
    /// accompaniments are still out, and they are out because they were never dealable,
    /// not because anybody answered them: this bound only ever removes.
    #[tokio::test]
    async fn answering_everything_dealable_deals_an_empty_deck() {
        let conn = dinner_conn().await;
        let plan = continuing(MealType::Dinner, "plan", "mel");
        for id in ["blank", "pancakes", "stew", "uncategorised"] {
            answered(&conn, "plan", id, "mel", true).await;
        }
        let deck = load_corpus(&conn, &plan).await.unwrap();
        assert!(
            ids(&deck).is_empty(),
            "everything this round could deal has been answered: {:?}",
            ids(&deck)
        );
        // And the four the round never dealt are still refused for their own reason —
        // `trifle` was not answered, it is a stated dessert (#184/#114).
        let fresh = load_corpus(&conn, &continuing(MealType::Dinner, "plan", "kit"))
            .await
            .unwrap();
        for accompaniment in ["brownie", "chips", "soup", "trifle"] {
            assert!(
                !ids(&fresh).contains(&accompaniment),
                "{accompaniment} is out of the round on #184's rule, answered or not"
            );
        }
    }

    /// **Finishing is not final** (#202): the deal is recomputed on every walk, so a
    /// recipe becoming dealable mid-plan un-finishes it with nothing to invalidate.
    ///
    /// Both halves are pinned in order, because they are two different rulings meeting.
    /// A dish *arriving* changes nothing — unread explicitly matches nothing, so #192
    /// keeps it out of every round. It is the **reading** landing that puts it in the
    /// deck, which is the same "running the worker is the act that delivers it" #193
    /// stated, seen from a member's side of the screen.
    #[tokio::test]
    async fn a_reading_landing_mid_plan_un_finishes_a_finished_deal() {
        let conn = dinner_conn().await;
        let plan = continuing(MealType::Dinner, "plan", "mel");
        for id in ["blank", "pancakes", "stew", "uncategorised"] {
            answered(&conn, "plan", id, "mel", true).await;
        }
        assert!(load_corpus(&conn, &plan).await.unwrap().len() == 0);

        // Ingest adds a dish while the plan is running. Nobody has read it.
        conn.execute(
            "INSERT INTO recipes (source, id, title, category)
             VALUES ('test', 'risotto', 'risotto', 'Miscellaneous')",
            (),
        )
        .await
        .unwrap();
        assert!(
            ids(&load_corpus(&conn, &plan).await.unwrap()).is_empty(),
            "an unread arrival explicitly matches nothing, so it deals to nobody (#192)"
        );

        // The meal-time worker reads it, and the very next walk deals it.
        read_as(&conn, "risotto", &[Sitting::Dinner]).await;
        assert_eq!(
            ids(&load_corpus(&conn, &plan).await.unwrap()),
            vec!["risotto"],
            "the deal recomputes, so there is no finished flag to go stale"
        );
    }

    /// The cap and this bound are two predicates, not one. `AND` binds tighter than
    /// `OR`, so a cap clause left unparenthesised would read as
    /// `cap IS NULL OR total_seconds IS NULL OR (fits AND unanswered)` — and every
    /// un-estimated recipe would be dealt forever, however often it had been answered.
    /// Every recipe here is un-estimated, which is the case that breaks.
    #[tokio::test]
    async fn an_answered_card_with_no_estimate_is_still_out_of_a_capped_deal() {
        let conn = dinner_conn().await;
        answered(&conn, "plan", "stew", "mel", true).await;
        let bounds = Bounds {
            max_total_seconds: Some(1800),
            ..continuing(MealType::Dinner, "plan", "mel")
        };
        let deck = load_corpus(&conn, &bounds).await.unwrap();
        assert_eq!(
            ids(&deck),
            vec!["blank", "pancakes", "uncategorised"],
            "the cap keeps un-estimated recipes (#80) and this still takes the answered one out"
        );
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
