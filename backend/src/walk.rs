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
//!
//! **The dice belong to the plan (#225).** The journey grammar below — the strategy, the
//! regions, the island rule, the teleports — is unchanged and stays pure over
//! `(corpus, rng)`. What changed is where that rng comes from: behind a plan minted
//! since #220 it is derived from the plan's seed, the person asking and the round
//! ([`deal_rng`]), so the deal is replayable, agrees between a phone and a laptop, and
//! can be reproduced after the fact. Behind no plan, or a plan older than the seed
//! column, it is still `StdRng::from_entropy()`.

use std::collections::{BTreeSet, HashMap, HashSet};

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
use sha2::{Digest, Sha256};

use crate::auth::CurrentUser;
use crate::session::MealType;
use crate::{error::AppError, AppState};

/// How many stops a walk has when the caller does not say. Long enough to feel
/// like a journey, short enough to render at a glance.
const DEFAULT_LEN: usize = 12;
/// A ceiling on `len`, so a caller cannot ask for an unbounded walk.
const MAX_LEN: usize = 30;
/// How many recipes one **round** of a seeded deal holds (#225).
///
/// A round is the unit the plan's seed deals in: [`deal_rng`] keys the journey on
/// `(plan seed, voter, round)`, and this is how far that journey runs before the next
/// round takes over. It is a **constant, not the caller's `len`**, and that is the whole
/// point — `len` is how much of the stream a device wants right now, so if it sized the
/// round then a phone asking for 12 and a laptop asking for 30 would be walking two
/// different streams and #225's "one voter, one deck in one order" would be false. It
/// equals [`MAX_LEN`] so the largest deal a client may ask for is usually served by one
/// round.
const ROUND_LEN: usize = MAX_LEN;
/// Domain separator for [`deal_rng`], so the deal's stream can never coincide with
/// another consumer of the same plan seed. The seed is shared by design (#212's
/// soundtrack is the other consumer today); the streams hung off it must not be.
const DEAL_DOMAIN: &[u8] = b"recipes/walk/deal/v1";

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
    /// What the **whole recipe** costs in kcal (#162), from `recipes.kcal`. `None` is
    /// unknown — the nutrition worker has not read this recipe, or read nothing it
    /// could weigh — never "free"; the client shows nothing for it, exactly as it does
    /// for an absent `total_seconds`.
    ///
    /// It is the whole-recipe total on purpose, with [`Self::servings`] beside it: per
    /// serving is a division the surface does, not a third number the wire could carry
    /// out of step with the two it came from (#162, `recipe_core::nutrition`).
    pub kcal: Option<i64>,
    /// Whether every ingredient line that stated a number was counted into
    /// [`Self::kcal`] — so whether that total is an estimate or only a floor
    /// (`recipes.kcal_complete`).
    ///
    /// The peer of [`Self::fully_timed`], and it rides on the card for the same reason:
    /// the browser has no ingredient readings here and could not run `recipe-core` over
    /// them if it did (no WASM). `false` → a line nothing could weigh counted as
    /// nothing, so the total can only be too low and the mark is `+`; `true` → the
    /// remaining error is ordinary estimation noise and the mark is `~`.
    pub kcal_complete: bool,
    /// How many people the recipe was read as feeding (`recipes.servings`). `None`
    /// until read — never `1`, because "we have not read this" and "this feeds one
    /// person" are different facts, and a per-serving figure that quietly assumed the
    /// second would be wrong by a factor of four on a tray of lasagne.
    ///
    /// Without it a bare total is uninterpretable, which is the whole reason #162 made
    /// the servings reading required rather than optional. The client shows no calorie
    /// figure at all when this is absent.
    pub servings: Option<i64>,
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
    /// The recipes this caller has already voted on in this plan (#202), **marked
    /// rather than missing** whenever the deal is replayable (#225).
    ///
    /// A marked recipe is in the graph — it can be hopped through, it counts toward
    /// every frequency — and it is simply not handed out. That is the difference
    /// between a deck you are working through and a deck that reshuffles under you:
    /// removing a row renumbers every [`RecipeId`] and resizes the start pool, so the
    /// very first `gen_range` lands somewhere else and the whole journey changes the
    /// moment you answer one card. Marked, the journey is a fixed function of the seed
    /// and the finished stops simply drop out of it, which is exactly what #225 asks
    /// of the replay.
    ///
    /// Empty for a walk with nothing to continue — no plan, or a plan whose seed
    /// predates #220 and whose deal is therefore not replayable at all. See
    /// [`load_corpus`] for why that second case keeps the rows out of the graph
    /// instead.
    answered: HashSet<RecipeId>,
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
            answered: HashSet::new(),
        }
    }

    /// Number of recipes in the corpus.
    fn len(&self) -> usize {
        self.cards.len()
    }

    /// How many recipes are still unanswered — the most a deal could ever hand out.
    /// The round loop reads it to stop the moment there is nothing left to find,
    /// rather than walking journey after journey over a deck that is spent.
    fn unanswered(&self) -> usize {
        self.cards.len() - self.answered.len()
    }

    /// Turn one of the walk's opaque landings back into a card the client can render.
    fn stop(&self, (recipe, via): Hop) -> Stop {
        Stop {
            via: via.and_then(|i| self.ingredient_names.get(i.0 as usize).cloned()),
            recipe: self.cards[recipe.0 as usize].clone(),
        }
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
    journey(corpus, len, rng, &HashSet::new())
        .into_iter()
        .map(|hop| corpus.stop(hop))
        .collect()
}

/// One landing of a journey, before it is turned into a renderable [`Stop`]: the recipe
/// reached, and the ingredient crossed to reach it (`None` for a start or a teleport).
type Hop = (RecipeId, Option<IngredientId>);

/// [`wander`]'s body, in the walk's own opaque ids, plus the one thing a multi-round
/// deal needs of it: `already` is what earlier rounds have handed out, and this journey
/// never lands there.
///
/// It is the same journey grammar either way — same strategy, same self-avoiding walk,
/// same island rule, same teleports — because "already dealt" is fed to the *same hard
/// visited set* the walk uses for "already visited on this leg"
/// ([`recipe_walk::WalkState::self_avoiding_beyond`]). Nothing about how a journey is
/// composed changes; what changes is where it is allowed to begin and land, and that is
/// what makes rounds partition the deck instead of overlapping it (see [`deal`]).
fn journey<R: RngCore>(
    corpus: &Corpus,
    len: usize,
    rng: &mut R,
    already: &HashSet<RecipeId>,
) -> Vec<Hop> {
    if corpus.len() == 0 {
        return Vec::new();
    }
    let strategy = TabuWeighted::default();

    // Teleport candidates: connected recipes (a leg can actually wander from them).
    // Fall back to every recipe only if nothing is connected, so an edgeless corpus
    // still yields stops rather than nothing. Both pools drop what earlier rounds
    // already dealt — a journey that started or teleported there would spend its stop
    // on a card this caller has already been handed.
    let fresh_of = |pool: Vec<RecipeId>| -> Vec<RecipeId> {
        pool.into_iter().filter(|r| !already.contains(r)).collect()
    };
    let connected = fresh_of(corpus.connected_starts());
    let all: Vec<RecipeId> = fresh_of((0..corpus.len() as u32).map(RecipeId).collect());
    let start_pool: &[RecipeId] = if connected.is_empty() {
        &all
    } else {
        &connected
    };
    // Nothing left to start on: every recipe is already dealt, which is the honest end
    // of the stream rather than a reason to deal one twice.
    if start_pool.is_empty() {
        return Vec::new();
    }

    // The first start, then a self-avoiding walk that owns the visited set. `&mut
    // *rng` reborrows the caller's stream so the start, every hop, and every
    // teleport all draw from the one sequence — a whole journey deterministic in one
    // seed.
    let start = start_pool[rng.gen_range(0..start_pool.len())];
    let mut hops: Vec<Hop> = vec![(start, None)];
    let mut walk = Walk::self_avoiding_beyond(
        &corpus.graph,
        &strategy,
        &mut *rng,
        start,
        len,
        already.clone(),
    );

    while hops.len() < len {
        // Wander until this region's frontier is spent (the walk yields `None`).
        while hops.len() < len {
            let Some(step) = walk.next() else { break };
            hops.push((step.recipe, Some(step.via)));
        }
        if hops.len() >= len {
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
        hops.push((fresh, None));
    }
    hops
}

/// The rng a round of the deal runs on: a stable function of the **plan's seed, the
/// voter, and the round** (#225), and of nothing else.
///
/// This is an API of the plan's universe now, not an implementation detail — a hand can
/// be reproduced from these three numbers alone, which is what makes "why was I dealt
/// this" answerable — so the function is spelled out rather than left to whatever
/// `StdRng` happens to do with a tuple:
///
/// ```text
/// key    = SHA-256( DEAL_DOMAIN ‖ seed_be64 ‖ voter_utf8 ‖ round_be32 )
/// stream = StdRng::from_seed(key)                      // ChaCha12, 32-byte seed
/// ```
///
/// - **The whole seed, big-endian, all eight bytes.** The column is an `INTEGER` below
///   2^53 (migration 0031) and every bit of it goes in. Folding or truncating it would
///   shrink the space of decks silently — two plans differing only in the discarded bits
///   would deal one deck while looking perfectly random.
/// - **Injective by construction.** One variable-length field, fixed-width fields either
///   side of it: two different triples cannot produce the same bytes, because equal
///   encodings force equal lengths, which forces the same voter id and so the same seed
///   and round. No two plans, people or rounds can collide onto one stream by accident.
/// - **Domain-separated.** [`DEAL_DOMAIN`] fences this stream off from every other
///   consumer of the same plan seed. The seed is *meant* to be shared — that is the
///   ruling it exists for, and the soundtrack derives its own running order from the
///   same number — so keeping the streams apart is this function's job, not the seed's.
/// - **Round is last and hashed**, so round 1 is a different journey rather than round 0
///   advanced by a few draws — a refill is a new deal, not the tail of the old one.
/// - **SHA-256 with no KDF**, the same call this repo already makes for session secrets
///   (`auth.rs`): nothing here is a password, and a shared seed is deliberately not a
///   credential (#225 keeps invite codes and login secrets on `OsRng`).
///
/// Changing any of it re-deals every plan in flight, so it is versioned in the domain
/// string rather than edited in place.
fn deal_rng(seed: i64, voter: &str, round: u32) -> StdRng {
    StdRng::from_seed(deal_key(seed, voter, round))
}

/// The 32 bytes [`deal_rng`] seeds its stream from — the written derivation on its own,
/// so it can be checked against a hash computed outside this program rather than only
/// against what this program did last time.
fn deal_key(seed: i64, voter: &str, round: u32) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(DEAL_DOMAIN);
    hash.update(seed.to_be_bytes());
    hash.update(voter.as_bytes());
    hash.update(round.to_be_bytes());
    hash.finalize().into()
}

/// Deal up to `len` cards off the plan's seed (#225) — the replayable deal.
///
/// # The stream, and where the round lives
///
/// A voter's deck in a plan is the rounds `0, 1, 2, …` laid end to end. Round `r` is
/// [`journey`] over the plan's corpus on [`deal_rng`]`(seed, voter, r)`, beyond
/// everything rounds `< r` landed on — so the rounds **partition** the deck rather than
/// overlapping it, and every recipe is reached by exactly one of them. A deal walks that
/// stream from the front and hands back the first `len` cards this voter has not
/// answered.
///
/// **The round is derived, and it is derived by replay** — it is nowhere in the
/// database, in the query string, or on the wire. `votes` already records what this
/// person answered; how far down the stream they are *is* that record read forward, so
/// deriving it makes the two impossible to disagree. A stored counter would be a second
/// writer for the same fact (and a write on a `GET`), free to drift from the votes that
/// actually decide what is left — the same reason a timer's deadline (0027) and a
/// per-serving calorie figure (#162) are derived rather than stored. A client-supplied
/// round would be worse still: a device that guessed wrong would deal itself a hand it
/// had already answered and call it fresh.
///
/// **A refill is round + 1, never a re-roll of round 0.** Answer everything round 0
/// dealt and its whole contribution drops out, so the first card of the next deal comes
/// from round 1's journey — a genuinely different order, not the old order with the
/// answered cards missing and the new arrivals inserted.
///
/// # What replay means, exactly
///
/// Same `(seed, voter, round)` over the same corpus snapshot ⇒ byte-for-byte the same
/// journey. Two devices agree because both compute it, and a device that comes back on
/// day four re-derives the deck it left rather than starting over: **answering a card
/// removes that card and moves nothing else**, because the answered set is a mark on the
/// corpus, never a hole in it (see [`Corpus::answered`]).
///
/// It is **exact over a corpus snapshot and best-effort after that**, and the docs say
/// so rather than promising more. Ingest and enrichment move the corpus under every
/// plan — a recipe arriving, a meal-time reading landing (#193), the kitchen's equipment
/// being recorded (#82) — and any of those changes the graph the journey runs over. That
/// is the honest boundary: the seed pins the dice, not the deck.
///
/// # Termination
///
/// Each round lands on at least its own start, which is drawn from what no earlier round
/// has dealt, so `dealt` grows every iteration and the loop is bounded by the corpus.
/// It stops as soon as it has `len` cards or has surfaced every unanswered one — so the
/// common deal is a single journey, and a plan whose deck is spent answers empty at once
/// rather than walking the corpus looking for a card that is not there.
fn deal(corpus: &Corpus, seed: i64, voter: &str, len: usize) -> Vec<Stop> {
    let mut stops: Vec<Stop> = Vec::new();
    let mut dealt: HashSet<RecipeId> = HashSet::new();
    let unanswered = corpus.unanswered();
    let mut round: u32 = 0;

    while stops.len() < len && stops.len() < unanswered && dealt.len() < corpus.len() {
        let mut rng = deal_rng(seed, voter, round);
        let hops = journey(corpus, ROUND_LEN, &mut rng, &dealt);
        if hops.is_empty() {
            break;
        }
        for hop in hops {
            dealt.insert(hop.0);
            // The finished stops drop out here, and only here: the journey above ran
            // over the whole corpus and does not know or care what has been answered.
            if !corpus.answered.contains(&hop.0) && stops.len() < len {
                stops.push(corpus.stop(hop));
            }
        }
        round += 1;
    }
    stops
}

/// What a walk is bounded to, already resolved against the database. [`Default`] is
/// unbounded — the whole corpus, which is what a walk with no channel gets.
#[derive(Debug, Clone, Default)]
struct Bounds {
    /// The pick session's time cap in seconds (#80); `None` = "Any".
    max_total_seconds: Option<i64>,
    /// The plan's calorie range, in **kcal a serving** (#213) — the number the card
    /// shows, not `recipes.kcal`'s whole-recipe total. `None` at either end is an open
    /// end; both `None` is "Any" and bounds nothing at all, which is what every plan is
    /// born as and what [`Default`] gives the plan-less walk.
    ///
    /// Two plain `Option`s, not an `Option<Range>`: "no range" must have exactly one
    /// representation, because the whole strictness rule keys off whether a range is set
    /// at all (see [`load_corpus`]).
    min_kcal_per_serving: Option<i64>,
    max_kcal_per_serving: Option<i64>,
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
    /// The plan's seed (migration 0031) — the one number all of a plan's shared
    /// randomness dangles off, and here the one that makes the deal replayable (#225).
    ///
    /// `Some` is a plan minted since the column landed, and its deal is
    /// `(seed, voter, round)` (see [`deal`]). `None` is one of two honest states, and
    /// they behave identically:
    ///
    /// - a walk with **no plan** behind it — there is no shared universe to hang a roll
    ///   off, and freshness is all this walk ever promised (#47);
    /// - a plan **older than the seed column**. It is not backfilled: minting a seed
    ///   today would claim its past deals were reproducible, and they were not. The
    ///   honest fallback is the entropy deal it has always had.
    seed: Option<i64>,
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
/// # The calorie range (#213)
///
/// A plan can say how big a serving it is planning, and the deck respects it. This sits
/// in SQL beside the cap, for the cap's reason: it is a scalar comparison SQL does
/// natively, over columns the row already carries.
///
/// **It reads the number the card shows, and derives it the way the card derives it.**
/// `recipes.kcal` is the *whole-recipe* total and `recipes.servings` is what that total
/// feeds; per serving is a division the surface does (#162), so a range stated in
/// whole-recipe kcal would bound a number no cook ever sees. The derivation here is
/// therefore `recipes.kcal / recipes.servings`, and it is `$lib/nutrition.formatCalories`
/// line for line:
///
/// ```text
/// formatCalories (the badge)                 load_corpus (this filter)
/// ──────────────────────────                 ─────────────────────────
/// kcal == null → null                        kcal IS NOT NULL
/// kcal <= 0    → null                        kcal > 0
/// servings == null → null                    servings IS NOT NULL
/// servings <= 0    → null                    servings > 0
/// each = Math.floor(kcal / servings)         kcal / servings   (integer division;
/// each < 1 → null                            kcal / servings >= 1
/// ```
///
/// SQLite's `/` on two INTEGERs truncates toward zero, and both operands are positive by
/// the lines above, so it **is** `Math.floor` — the two cannot disagree about a card.
/// Every early `null` in the badge is a case where there is no per-serving number at
/// all, and a recipe with no number cannot explicitly fit a range, so each maps to an
/// exclusion here rather than to a special case.
///
/// **Strict, by ruling (#193).** With a range set, a recipe is dealt only when its
/// reading explicitly fits, and that includes `kcal_complete = 1`:
///
/// - `kcal_complete = 0` means at least one line stated a number nothing could weigh, so
///   the total is a **floor** (#162). A floor **above** the max proves the recipe does
///   not fit; a floor **below** the max proves nothing at all — the real dish could be
///   anywhere above it. Only one of those two is a proof, so an incomplete reading is
///   dealt in neither case, and the clause that says so is `kcal_complete = 1`.
/// - An unread recipe (`kcal IS NULL`) is out for the same reason it is out of a meal
///   round: missing data is enrichment work (#162's worker), never a reason to widen the
///   filter. The deck honestly thins, and thinning is the pressure that gets the corpus
///   read.
///
/// This is the **opposite** call to the time cap directly above, and the difference is
/// the same one the kitchen bound draws: a time estimate is a lower bound *even when we
/// have it*, so the cap is choosing between two flavours of uncertainty and has no
/// reason to be stricter with one — whereas `kcal_complete` tells us exactly which
/// readings are complete, so there is a proof to insist on and no reason to accept a
/// guess beside it.
///
/// **No range set changes nothing.** Both parameters `NULL` short-circuits the whole
/// clause, so an unbounded walk, a plan-less walk and every plan that exists today deal
/// exactly what they deal now. That is why the columns have no default (migration 0030):
/// a plan born inside a range would silently thin its own deck to whatever the nutrition
/// worker had reached.
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
/// So a meal round never deals the `(source, id)`s this caller has already voted on **in
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
/// The predicate is `?2 IS NULL OR …`, so a plan-less walk — which carries no channel and
/// therefore no voter — is untouched, exactly like the meal bound above. Note the cap's
/// clause is parenthesised: `AND` binds tighter than `OR`, so without the brackets this
/// would have attached to `total_seconds <= ?1` alone and left every un-estimated recipe
/// dealt forever.
///
/// ## Answered is a *mark* once the deal is replayable (#225)
///
/// The question stays in SQL for every reason above — it is the same covered lookup on
/// `votes`' primary key, asked once per row, and there is still no second query and no
/// index rebuilt in memory. What changed is where the answer is *applied*:
///
/// - **A seeded plan marks the row** ([`Corpus::answered`]) and [`deal`] drops the stop.
///   A deal replayable from `(seed, voter, round)` is only replayable if the corpus it
///   runs over holds still: dropping a row renumbers every [`RecipeId`] and resizes the
///   start pool, so the first draw lands elsewhere and the entire journey changes the
///   moment somebody answers one card — which would make "a returning device re-derives
///   the identical journey" false in exactly the case #225 exists for. Marked, the
///   journey is fixed and the finished stops drop out of it.
/// - **A plan with no seed drops the row**, exactly as #202 built it. There is nothing
///   to replay there, so there is nothing to hold still, and the narrower graph is
///   strictly better: it is what the entropy deal has always walked.
///
/// One query serves both; the difference is one `continue` below, beside the meal and
/// kitchen filters it now reads like.
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
            "SELECT source, id, title, image, category, area, total_seconds, fully_timed, ingredients, equipment, sittings, kcal, kcal_complete, servings,
                    (?2 IS NOT NULL AND EXISTS (
                       SELECT 1 FROM votes
                        WHERE votes.channel_id = ?2
                          AND votes.voter_id = ?3
                          AND votes.source = recipes.source
                          AND votes.id = recipes.id)) AS answered
             FROM recipes
             WHERE (?1 IS NULL OR total_seconds IS NULL OR total_seconds <= ?1)
               AND (
                     -- No range set: both ends open, so this bounds nothing and the
                     -- deck is exactly today's (#213).
                     (?4 IS NULL AND ?5 IS NULL)
                     -- A range IS set, so only an explicit fit is dealt (#193, ruled).
                     -- `kcal / servings` is the number the card shows, derived here the
                     -- way `$lib/nutrition.formatCalories` derives it: integer division
                     -- over two positive integers is its `Math.floor`, and each of its
                     -- early `null` exits is a line below. `kcal_complete = 1` is the
                     -- ruling itself — an incomplete total is a floor, and a floor below
                     -- the max proves nothing.
                     OR (recipes.kcal_complete = 1
                         AND recipes.kcal IS NOT NULL AND recipes.kcal > 0
                         AND recipes.servings IS NOT NULL AND recipes.servings > 0
                         AND recipes.kcal / recipes.servings >= 1
                         AND (?4 IS NULL OR recipes.kcal / recipes.servings >= ?4)
                         AND (?5 IS NULL OR recipes.kcal / recipes.servings <= ?5))
                   )",
            libsql::params![
                bounds.max_total_seconds,
                channel,
                voter,
                bounds.min_kcal_per_serving,
                bounds.max_kcal_per_serving
            ],
        )
        .await?;

    let mut out: Vec<(RecipeCard, Vec<String>)> = Vec::new();
    let mut answered_ids: HashSet<RecipeId> = HashSet::new();
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
            // The calorie estimate rides out the same way the time estimate does
            // (#162/#84) — the walk is already reading this row, so it is one more
            // column rather than a second read. New columns go on the **end** of the
            // SELECT, never in the middle: every read here is positional, so inserting
            // one above would silently re-point all of them (the #109 outage).
            kcal: row.get::<Option<i64>>(11)?,
            // NOT NULL DEFAULT 0 like `fully_timed`, so a plain read with no absent
            // case: an unread row is `0`, and its `kcal` is NULL anyway.
            kcal_complete: row.get::<i64>(12)? != 0,
            servings: row.get::<Option<i64>>(13)?,
        };
        // What this caller has already answered in this plan (#202). `0` whenever no
        // voter was supplied, so a plan-less walk reads it and is told nothing.
        let answered = row.get::<i64>(14)? != 0;
        // Without a seed there is no journey to hold still, so the answered row never
        // enters the graph at all — #202's deal, unchanged (see this function's doc).
        if answered && bounds.seed.is_none() {
            continue;
        }
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
        // The mark is recorded against the id this row is about to get, which is its
        // position — the same mapping `Corpus::build` uses, and the reason it is taken
        // here rather than recomputed from the cards afterwards.
        if answered {
            answered_ids.insert(RecipeId(out.len() as u32));
        }
        out.push((card, names));
    }

    let mut corpus = Corpus::build(out);
    corpus.answered = answered_ids;
    Ok(corpus)
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
        // Read fresh off the plan on every walk, like the cap: the client passes only the
        // channel, so what bounds the deck is what the plan currently says (#213/#80).
        min_kcal_per_serving: plan.min_kcal_per_serving,
        max_kcal_per_serving: plan.max_kcal_per_serving,
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
        // The plan's own seed (#220), which is what the deal hangs its dice off (#225).
        // Read per walk beside the rest of the bounds — it is a column on the same row,
        // so it costs nothing extra — and `None` for a plan minted before the column
        // existed, which keeps its entropy deal.
        seed: plan.seed,
    })
}

/// `GET /api/walk?len=<n>&channel=<pick>` — a fresh variety-first walk over the
/// corpus, bounded to the pick session's time cap (#80), to its calorie range (#213),
/// to what its kitchen can make (#82), and to the meal it is for (#184) whenever a
/// channel is named.
///
/// Session-gated like every person-facing route (#25).
///
/// The session is *read* as well as required: a meal round never re-deals what this
/// caller has already answered in this plan (#202), so a plan that runs for days is
/// continued rather than restarted.
///
/// **The dice come from the plan, not the machine (#225).** A plan minted since #220
/// carries a seed — the one number every shared roll in a meal hangs off — and the deal
/// is a function of `(seed, voter, round)` (see [`deal`]). A reload re-derives the deck
/// it left; a phone and a laptop walk one deck in one order; and a hand can be
/// reproduced after the fact from three numbers, which is the whole of "why was I dealt
/// this".
///
/// A plan **without** a seed — one older than the column, or no plan at all — keeps the
/// entropy deal it has always had. That is the honest state, not a gap to paper over: a
/// seed minted today would claim those plans' past deals were reproducible, and they
/// were not.
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
    // let a client deal itself the cards somebody else has left (#202), and it is half
    // of the deal's key (#225), which no query string may name either.
    let bounds = resolve_bounds(&state, params.channel.as_deref(), &user.telegram_user_id).await?;
    let bounds = &bounds;
    let corpus = state
        .with_db(move |conn| async move { load_corpus(&conn, bounds).await })
        .await
        .map_err(|e| AppError::Internal(format!("could not load the corpus: {e:#}")))?;
    Ok(Json(WalkResponse {
        stops: stops_for(&corpus, bounds, len),
    }))
}

/// Which deal these bounds get: the plan's own, or the machine's.
///
/// Both halves of the key have to be in hand — the plan's seed and the person asking —
/// and they arrive together, because a channelled walk is always somebody's round
/// (#202) and a seed is always a plan's. Anything else is the entropy deal: no plan at
/// all (#47's wander, which promises nothing but freshness), or a plan minted before the
/// seed column, whose deals never were reproducible and are not going to be told they
/// were (#225).
///
/// A function rather than a `match` inside the handler so the choice itself is testable
/// without an `AppState` — it is a ruling about which plans are replayable, not
/// plumbing.
fn stops_for(corpus: &Corpus, bounds: &Bounds, len: usize) -> Vec<Stop> {
    match (&bounds.seed, &bounds.answered_by) {
        (Some(seed), Some(who)) => deal(corpus, *seed, &who.voter, len),
        _ => wander(corpus, len, &mut StdRng::from_entropy()),
    }
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
            kcal: None,
            kcal_complete: false,
            servings: None,
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

    // ---- the deal comes off the plan's seed (#225) --------------------------

    /// Two plan seeds of the shape migration 0031 mints: non-negative, below 2^53, and
    /// with a non-zero high half — the half a derivation that truncated the column to a
    /// 32-bit PRNG state would throw away.
    const SEED_A: i64 = 0x0000_1122_3344_5566;
    const SEED_B: i64 = 0x0000_199A_ABBC_CDDE;

    /// The recipe ids a deal hands over, **in the order it dealt them**. Order is the
    /// point of nearly every test below, so nothing here sorts.
    fn hand(corpus: &Corpus, seed: i64, voter: &str, len: usize) -> Vec<String> {
        deal(corpus, seed, voter, len)
            .into_iter()
            .map(|s| s.recipe.id)
            .collect()
    }

    /// Mark `ids` as answered by this caller — what [`load_corpus`] does from `votes`.
    fn mark_answered(corpus: &mut Corpus, ids: &[&str]) {
        for id in ids {
            let at = corpus
                .cards
                .iter()
                .position(|c| c.id == *id)
                .expect("marking a recipe that is in the corpus");
            corpus.answered.insert(RecipeId(at as u32));
        }
    }

    /// **The derivation, pinned to its written form.** `deal_rng` is an API of the
    /// plan's universe — a hand is meant to be reproducible from three numbers — so the
    /// exact bytes that go into the hash are part of the contract, not an implementation
    /// detail free to drift.
    ///
    /// The expected value is **not** a note of what this code did last time: it is
    /// `sha256(b"recipes/walk/deal/v1" + seed.to_be_bytes(8) + b"voter-1" + 0u32.be)`
    /// computed outside this program, so the assertion is against the derivation as
    /// written down rather than against the implementation of it. The domain string, the
    /// field order, the endianness of both numbers and the hash itself are all in that
    /// one line; any change to any of them re-deals every plan in flight, which is why
    /// this is meant to be awkward to change — it moves only alongside a new version in
    /// [`DEAL_DOMAIN`].
    #[test]
    fn the_deal_derivation_is_pinned_to_its_written_form() {
        assert_eq!(
            hex::encode(deal_key(SEED_A, "voter-1", 0)),
            "68edded71d9c07d735a9de2d9ef5c79a2bb38a691a3e3972cb4a7f336dabed59"
        );
        // …and the stream really is seeded from those bytes.
        assert_eq!(
            deal_rng(SEED_A, "voter-1", 0).gen::<u64>(),
            StdRng::from_seed(deal_key(SEED_A, "voter-1", 0)).gen::<u64>()
        );
    }

    /// Every part of the key is *load-bearing*: change one and the stream changes.
    /// Dropping any of the three from the hash — which is the whole failure mode this
    /// guards — leaves one of these pairs equal.
    #[test]
    fn every_part_of_the_deal_key_changes_the_stream() {
        let draw = |seed: i64, voter: &str, round: u32| deal_rng(seed, voter, round).gen::<u64>();
        let base = draw(SEED_A, "mel", 0);
        assert_ne!(base, draw(SEED_B, "mel", 0), "the plan's seed is in it");
        assert_ne!(base, draw(SEED_A, "kit", 0), "the voter is in it");
        assert_ne!(base, draw(SEED_A, "mel", 1), "the round is in it");
    }

    /// **The whole seed is in the key, not the low half of it.** Two plans whose seeds
    /// differ only above 2^32 are different plans, and truncating the column to fit a
    /// 32-bit state — the cheap way to seed a PRNG from a number — would deal them one
    /// deck while looking perfectly random. Eight such pairs, so it cannot pass by luck.
    #[test]
    fn the_whole_width_of_the_seed_is_in_the_key() {
        for low in 0..8i64 {
            let below = deal_rng(low, "mel", 0).gen::<u64>();
            let above = deal_rng(low + (1 << 32), "mel", 0).gen::<u64>();
            assert_ne!(below, above, "seeds differing only above 2^32: {low}");
        }
    }

    /// **Replayable.** The same `(seed, voter, round)` over the same corpus deals the
    /// same journey, stop for stop — `via` threads and all, not merely the same set of
    /// recipes. This is what a returning device re-derives.
    #[test]
    fn the_same_seed_and_voter_deal_the_identical_journey_twice() {
        let corpus = ring_corpus(40);
        let first = deal(&corpus, SEED_A, "mel", 12);
        let second = deal(&corpus, SEED_A, "mel", 12);
        assert_eq!(first.len(), 12);
        assert_eq!(first, second, "byte-for-byte the same journey");
    }

    /// One deck per person: two members of the same plan are dealt different journeys,
    /// so a plan is not one shared shuffle everybody swipes in lockstep.
    #[test]
    fn a_different_voter_is_dealt_a_different_journey() {
        let corpus = ring_corpus(40);
        assert_ne!(
            hand(&corpus, SEED_A, "mel", 12),
            hand(&corpus, SEED_A, "kit", 12)
        );
    }

    /// One deck per plan: the same person in two plans is dealt two decks. Without the
    /// seed in the key, every plan a person joined would deal them the same order.
    #[test]
    fn a_different_plan_seed_is_a_different_journey() {
        let corpus = ring_corpus(40);
        assert_ne!(
            hand(&corpus, SEED_A, "mel", 12),
            hand(&corpus, SEED_B, "mel", 12)
        );
    }

    /// **Answering moves nothing but the card answered** — the replay composing with
    /// #202's exclusion, which is the whole reason the answered set is a mark on the
    /// corpus rather than a hole in it. Take a hand, answer one card in the middle of
    /// it, deal again: that card is gone and every other card is in the same place.
    ///
    /// Removing the row instead (the shape #202 shipped, before there was anything to
    /// replay) renumbers every `RecipeId` and resizes the start pool, so the first draw
    /// lands elsewhere and the journey is unrecognisable. That is what this fails on.
    #[test]
    fn answering_a_card_removes_it_and_moves_nothing_else() {
        let mut corpus = ring_corpus(40);
        let before = hand(&corpus, SEED_A, "mel", 12);
        let answered = before[5].clone();

        mark_answered(&mut corpus, &[answered.as_str()]);
        let after = hand(&corpus, SEED_A, "mel", 12);

        assert!(!after.contains(&answered), "the answered card is gone");
        let expected: Vec<String> = before
            .iter()
            .filter(|id| **id != answered)
            .cloned()
            .collect();
        assert_eq!(
            after[..expected.len()],
            expected[..],
            "the rest of the deck kept its order"
        );
    }

    /// **A refill is round + 1.** Answer everything round 0 dealt and the next deal
    /// comes from round 1's journey — which is a different journey over the same corpus,
    /// not round 0 re-rolled with the answered cards missing and the arrivals since
    /// slotted in. The two rounds are *disjoint*, which is the strongest statement of it:
    /// the rounds partition the deck, so a refill is genuinely further along the stream.
    #[test]
    fn a_refill_after_a_round_is_answered_comes_from_the_next_round() {
        let mut corpus = ring_corpus(40);
        // A round holds ROUND_LEN cards, and this corpus is bigger than one round, so
        // there is a round 1 to reach.
        let round_0 = hand(&corpus, SEED_A, "mel", ROUND_LEN);
        assert_eq!(round_0.len(), ROUND_LEN);

        let answered: Vec<&str> = round_0.iter().map(String::as_str).collect();
        mark_answered(&mut corpus, &answered);
        let round_1 = hand(&corpus, SEED_A, "mel", ROUND_LEN);

        assert_eq!(round_1.len(), 40 - ROUND_LEN, "what the ring has left");
        for id in &round_1 {
            assert!(
                !round_0.contains(id),
                "{id} was already dealt in round 0, so it is not a refill"
            );
        }
    }

    /// …and the refill really is the **next round's stream**, not the first round's rng
    /// run again over what is left.
    ///
    /// Disjointness above cannot tell those two apart — the rounds partition the deck
    /// either way — and the difference is the whole of "round advances the stream": a
    /// deal whose round never moved would keep re-rolling one order forever, which is
    /// what the issue rules out for a refill after new recipes arrive.
    #[test]
    fn a_refill_runs_the_next_rounds_stream_not_the_first_ones_again() {
        let mut corpus = ring_corpus(70);
        let round_0 = hand(&corpus, SEED_A, "mel", ROUND_LEN);
        let answered: Vec<&str> = round_0.iter().map(String::as_str).collect();
        mark_answered(&mut corpus, &answered);
        let refill = hand(&corpus, SEED_A, "mel", ROUND_LEN);

        // Everything round 0's journey landed on — what round 1 is dealt *beyond*.
        let landed: HashSet<RecipeId> = journey(
            &corpus,
            ROUND_LEN,
            &mut deal_rng(SEED_A, "mel", 0),
            &HashSet::new(),
        )
        .into_iter()
        .map(|hop| hop.0)
        .collect();
        let stream_of = |round: u32| -> Vec<String> {
            journey(
                &corpus,
                ROUND_LEN,
                &mut deal_rng(SEED_A, "mel", round),
                &landed,
            )
            .into_iter()
            .map(|hop| corpus.cards[hop.0 .0 as usize].id.clone())
            .collect()
        };
        assert_eq!(refill, stream_of(1), "the refill is round 1's journey");
        assert_ne!(refill, stream_of(0), "…not round 0's, dealt a second time");
    }

    /// The rounds partition the deck **whatever anyone answered**: round 1's journey is
    /// beyond round 0's landings, not beyond the cards round 0 happened to hand over.
    /// Otherwise a vote would move where round 1 begins, and the stream would shift
    /// under the voter exactly as it did before #225.
    #[test]
    fn a_vote_does_not_move_where_the_next_round_begins() {
        let plain = ring_corpus(40);
        let mut voted = ring_corpus(40);
        let round_0 = hand(&plain, SEED_A, "mel", ROUND_LEN);
        // One card in round 0 is answered, and everything else is left alone.
        mark_answered(&mut voted, &[round_0[3].as_str()]);

        let all_plain = hand(&plain, SEED_A, "mel", 40);
        let all_voted = hand(&voted, SEED_A, "mel", 40);
        let expected: Vec<String> = all_plain
            .iter()
            .filter(|id| **id != round_0[3])
            .cloned()
            .collect();
        assert_eq!(all_voted, expected, "one card gone, the stream unmoved");
    }

    /// Answering the whole deck deals nothing, and does so **at once** rather than
    /// walking journey after journey over a corpus with nothing left in it.
    #[test]
    fn a_deck_that_is_wholly_answered_deals_nothing() {
        let mut corpus = ring_corpus(12);
        let all: Vec<String> = (0..12).map(|r| r.to_string()).collect();
        mark_answered(
            &mut corpus,
            &all.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        assert!(deal(&corpus, SEED_A, "mel", 12).is_empty());
    }

    /// The deal reaches every card eventually — asked for more than the corpus holds it
    /// hands over the whole thing, once each. A round that failed to partition would
    /// re-walk the same neighbourhood and come up short.
    #[test]
    fn the_rounds_between_them_reach_the_whole_deck() {
        let corpus = ring_corpus(70);
        let dealt = hand(&corpus, SEED_A, "mel", 70);
        assert_eq!(dealt.len(), 70, "every recipe, over three rounds");
        let distinct: HashSet<&String> = dealt.iter().collect();
        assert_eq!(distinct.len(), 70, "and none of them twice");
    }

    /// The bounds of a plan whose deal is replayable: a seed, and the person it is
    /// being dealt to.
    fn seeded(seed: i64, voter: &str) -> Bounds {
        Bounds {
            seed: Some(seed),
            answered_by: Some(Answered {
                channel: "plan".to_owned(),
                voter: voter.to_owned(),
            }),
            ..Bounds::default()
        }
    }

    /// **A seeded plan deals from its seed**, and says so where it counts: the same
    /// bounds over the same corpus hand back the same journey every time.
    #[test]
    fn a_seeded_plan_deals_the_same_journey_every_call() {
        let corpus = ring_corpus(60);
        let bounds = seeded(SEED_A, "mel");
        let first = stops_for(&corpus, &bounds, 12);
        for _ in 0..8 {
            assert_eq!(first, stops_for(&corpus, &bounds, 12));
        }
    }

    /// **A plan with no seed keeps the entropy deal** (#47), unchanged — the honest
    /// fallback for the plans that predate #220, which never had a reproducible deal to
    /// promise. Freshness is observable and replay is not, so this asserts the thing
    /// that is actually true of `from_entropy`: the journeys differ.
    ///
    /// Over a 60-recipe ring the first stop alone is a 1-in-60 draw, so eight deals
    /// coming out identical is not a run of bad luck — it is the seeded path having
    /// taken a plan that has no seed.
    #[test]
    fn a_plan_with_no_seed_deals_from_entropy() {
        let corpus = ring_corpus(60);
        let unseeded = Bounds {
            answered_by: Some(Answered {
                channel: "plan".to_owned(),
                voter: "mel".to_owned(),
            }),
            ..Bounds::default()
        };
        let first = stops_for(&corpus, &unseeded, 12);
        assert!(
            (0..8).any(|_| stops_for(&corpus, &unseeded, 12) != first),
            "an unseeded plan is dealt a fresh journey every time"
        );
    }

    /// A walk with **no plan** is nobody's round, so it is dealt from entropy too — a
    /// seed with no voter behind it is not half a key, it is not a key.
    #[test]
    fn a_walk_with_no_plan_deals_from_entropy() {
        let corpus = ring_corpus(60);
        let stray = Bounds {
            seed: Some(SEED_A),
            answered_by: None,
            ..Bounds::default()
        };
        let first = stops_for(&corpus, &stray, 12);
        assert!((0..8).any(|_| stops_for(&corpus, &stray, 12) != first));
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

    /// The calorie estimate rides out on the card too (#162), and all three of its
    /// columns do — the total, whether that total is complete, and the servings it is
    /// divided by. Dropping any one of them silently changes what the badge claims: no
    /// `servings` and a per-serving figure becomes a whole-recipe one, no
    /// `kcal_complete` and a floor renders as an estimate.
    ///
    /// It also pins the column order once more. The three reads are positional and sit
    /// at the end of the `SELECT`; a column inserted above them reads the wrong value
    /// rather than failing (#109), and here the wrong value is a plausible number.
    #[tokio::test]
    async fn a_loaded_card_carries_its_calorie_estimate() {
        let conn = nutrition_conn().await;
        let corpus = load_corpus(&conn, &Bounds::default()).await.unwrap();
        let card = |id: &str| {
            corpus
                .cards
                .iter()
                .find(|c| c.id == id)
                .expect("seeded recipe is in the corpus")
                .clone()
        };
        let counted = card("counted");
        assert_eq!(counted.kcal, Some(1045));
        assert_eq!(counted.servings, Some(4));
        assert!(counted.kcal_complete);

        let floored = card("floor");
        assert_eq!(floored.kcal, Some(900));
        assert_eq!(floored.servings, Some(2));
        assert!(
            !floored.kcal_complete,
            "a line nothing could weigh leaves the total a floor"
        );

        // Unread is unread: NULL, never 0 — a dish with no calories is never true of
        // food, and "feeds one" is not what "nobody has read this" means.
        let unread = card("unread");
        assert_eq!(unread.kcal, None);
        assert_eq!(unread.servings, None);
        assert!(!unread.kcal_complete);
    }

    /// A corpus shaped for the calorie badge (#162): a complete total, a total that is
    /// only a floor, and a recipe the nutrition worker has not reached.
    async fn nutrition_conn() -> libsql::Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        for (id, kcal, complete, servings) in [
            ("counted", Some(1045i64), 1i64, Some(4i64)),
            ("floor", Some(900i64), 0i64, Some(2i64)),
            ("unread", None, 0i64, None),
        ] {
            conn.execute(
                "INSERT INTO recipes (source, id, title, kcal, kcal_complete, servings)
                 VALUES ('test', ?1, ?1, ?2, ?3, ?4)",
                libsql::params![id, kcal, complete, servings],
            )
            .await
            .unwrap();
        }
        conn
    }

    // ---- the calorie range (#213) ------------------------------------------

    /// A corpus shaped for the calorie range: one recipe per relationship a reading can
    /// have with a bound over **kcal a serving**.
    ///
    /// Every total here is a whole-recipe total with a servings count beside it, as
    /// `recipes` stores them (#162), so the per-serving figure each row is about is a
    /// division and not a stored number — which is the thing under test. The right-hand
    /// column is worked out by hand, off `formatCalories`' rule (the floor of
    /// `kcal / servings`), and never by re-running the SQL:
    ///
    /// ```text
    /// id          kcal  complete  servings   a serving   what it is
    /// ─────────── ────  ────────  ────────   ─────────   ──────────────────────────
    /// salad       1240      1         4          310     light, counted in full
    /// lasagne     2810      1         4          702     2810/4 = 702.5, floors to 702
    /// feast       4800      1         4         1200     hearty, counted in full
    /// floor_low   1200      0         4          300     a floor UNDER a 500 max
    /// floor_high  6000      0         4         1500     a floor OVER a 500 max
    /// unread      NULL      0       NULL           —     the worker has not reached it
    /// rub           90      1       100            0     0.9 floors to 0 — no number
    /// ```
    ///
    /// `rub` is a big-batch spice rub: one weighable line, read as feeding a hundred. It
    /// is the row that makes `kcal / servings >= 1` a rule rather than a formality —
    /// `formatCalories` shows *nothing* for it, so it is a card the badge is silent
    /// about and no range can explicitly fit.
    async fn calorie_conn() -> libsql::Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        for (id, kcal, complete, servings) in [
            ("salad", Some(1240i64), 1i64, Some(4i64)),
            ("lasagne", Some(2810), 1, Some(4)),
            ("feast", Some(4800), 1, Some(4)),
            ("floor_low", Some(1200), 0, Some(4)),
            ("floor_high", Some(6000), 0, Some(4)),
            ("unread", None, 0, None),
            ("rub", Some(90), 1, Some(100)),
        ] {
            conn.execute(
                "INSERT INTO recipes (source, id, title, kcal, kcal_complete, servings)
                 VALUES ('test', ?1, ?1, ?2, ?3, ?4)",
                libsql::params![id, kcal, complete, servings],
            )
            .await
            .unwrap();
        }
        conn
    }

    /// Bounded by a calorie range alone (#213), in kcal a serving.
    fn ranged(min: Option<i64>, max: Option<i64>) -> Bounds {
        Bounds {
            min_kcal_per_serving: min,
            max_kcal_per_serving: max,
            ..Bounds::default()
        }
    }

    /// **No range set changes nothing.** Both ends open is "Any", which is what every
    /// plan is born as (migration 0030 gives the columns no default), and the deck is
    /// the whole corpus — the unread recipe and both floors included, exactly as they
    /// are dealt today.
    ///
    /// This is the guard on the strict rule below: strictness is the price of *asking*,
    /// and a plan that asked nothing must not pay it.
    #[tokio::test]
    async fn no_calorie_range_takes_the_whole_corpus() {
        let conn = calorie_conn().await;
        let corpus = load_corpus(&conn, &ranged(None, None)).await.unwrap();
        assert_eq!(
            ids(&corpus),
            vec![
                "feast",
                "floor_high",
                "floor_low",
                "lasagne",
                "rub",
                "salad",
                "unread"
            ]
        );
        // …and identical to the default bounds, so "Any" and "no plan" are one deck.
        assert_eq!(
            ids(&load_corpus(&conn, &Bounds::default()).await.unwrap()),
            ids(&corpus)
        );
    }

    /// A range deals the readings that explicitly fit it, and nothing else (#193).
    ///
    /// `salad` at 310 a serving is inside 200–800; `lasagne` at 702 is inside it;
    /// `feast` at 1200 is outside. The two floors and the unread recipe are out on the
    /// strictness rule, which the tests below pin one at a time.
    #[tokio::test]
    async fn a_calorie_range_deals_only_readings_that_fit_it() {
        let conn = calorie_conn().await;
        let corpus = load_corpus(&conn, &ranged(Some(200), Some(800)))
            .await
            .unwrap();
        assert_eq!(ids(&corpus), vec!["lasagne", "salad"]);
    }

    /// The bound is **inclusive at both ends**, the same call the time cap makes (a
    /// 30-minute recipe fits a 30-minute cap).
    ///
    /// Pinned on `lasagne`'s hand-computed 702 from both sides: a range that starts
    /// exactly there deals it, and one that ends exactly there deals it. A `>=`
    /// weakened to `>`, or a `<=` weakened to `<`, drops it from one of these two decks.
    #[tokio::test]
    async fn the_calorie_range_includes_both_of_its_ends() {
        let conn = calorie_conn().await;
        let from_exactly = load_corpus(&conn, &ranged(Some(702), Some(800)))
            .await
            .unwrap();
        assert_eq!(
            ids(&from_exactly),
            vec!["lasagne"],
            "a serving exactly at the min fits"
        );
        let to_exactly = load_corpus(&conn, &ranged(Some(400), Some(702)))
            .await
            .unwrap();
        assert_eq!(
            ids(&to_exactly),
            vec!["lasagne"],
            "a serving exactly at the max fits"
        );
    }

    /// **The strict rule, by name (#193).** An incomplete reading whose floor sits
    /// *below* the max is **not dealt**.
    ///
    /// `floor_low` is 1200 kcal over 4 servings — 300 a serving — with
    /// `kcal_complete = 0`, so at least one line stated a number nothing could weigh and
    /// 300 is the least it can be. A range of "up to 500 a serving" therefore proves
    /// nothing about it: the real dish is somewhere at or above 300 and could be 900.
    /// Admitting it would put a guess in a deck that promised a proof.
    ///
    /// `salad` sits at 310 in the very same window with a complete reading and *is*
    /// dealt, which is what makes this a test about completeness rather than about the
    /// number.
    #[tokio::test]
    async fn an_incomplete_reading_below_the_max_is_not_dealt_while_a_range_is_set() {
        let conn = calorie_conn().await;
        let corpus = load_corpus(&conn, &ranged(None, Some(500))).await.unwrap();
        assert!(
            !ids(&corpus).contains(&"floor_low"),
            "a floor below the max proves nothing, so it is not dealt: {:?}",
            ids(&corpus)
        );
        assert!(
            ids(&corpus).contains(&"salad"),
            "the same window with a complete reading is dealt: {:?}",
            ids(&corpus)
        );
    }

    /// The other side of the same rule: an incomplete reading whose floor is already
    /// *above* the max is out too — there the exclusion really is a proof, and the deck
    /// looks the same either way, which is exactly why the test above is the one that
    /// carries the ruling.
    #[tokio::test]
    async fn an_incomplete_reading_above_the_max_is_not_dealt_either() {
        let conn = calorie_conn().await;
        let corpus = load_corpus(&conn, &ranged(None, Some(500))).await.unwrap();
        assert!(
            !ids(&corpus).contains(&"floor_high"),
            "a floor already over the max cannot fit: {:?}",
            ids(&corpus)
        );
    }

    /// An unread recipe is in no ranged deck — #162's worker has not reached it — the
    /// same treatment the meal round gives an unread sitting and for the same written
    /// reason: missing data is enrichment work, never a reason to widen the filter.
    ///
    /// A range as wide as the API allows still leaves it out, so this is about the
    /// absence and not about the number.
    #[tokio::test]
    async fn an_unread_recipe_is_not_dealt_while_a_range_is_set() {
        let conn = calorie_conn().await;
        let corpus = load_corpus(&conn, &ranged(Some(1), Some(10_000)))
            .await
            .unwrap();
        assert_eq!(
            ids(&corpus),
            vec!["feast", "lasagne", "salad"],
            "only the three complete readings, however wide the range"
        );
    }

    /// **The range reads kcal a *serving*, not the whole-recipe total**, and derives it
    /// the way the card does.
    ///
    /// `lasagne` stores 2810 kcal over 4 servings. Worked out by hand off
    /// `formatCalories`' rule, that is `floor(2810 / 4) = 702` a serving — the number
    /// the badge prints. So:
    ///
    /// - a range of 700–800 **deals** it, because 702 is in it;
    /// - a range of 2500–3000 deals **nothing**, even though 2810 is squarely inside —
    ///   which is only true if the filter divides.
    ///
    /// The second half is the point: it fails on any implementation that compares the
    /// stored total, and passes only on one that compares what the cook is shown.
    #[tokio::test]
    async fn the_range_reads_kcal_a_serving_not_the_whole_recipe_total() {
        let conn = calorie_conn().await;
        assert_eq!(
            ids(&load_corpus(&conn, &ranged(Some(700), Some(800)))
                .await
                .unwrap()),
            vec!["lasagne"],
            "702 a serving is in a 700-800 range"
        );
        assert!(
            ids(&load_corpus(&conn, &ranged(Some(2500), Some(3000)))
                .await
                .unwrap())
            .is_empty(),
            "2810 is the tray, not the plate — nothing is dealt by the total"
        );
    }

    /// The division floors, it does not round: `2810 / 4` is 702.5 and the card prints
    /// 702, so a range pinned at 702 holds this recipe and one pinned at 703 does not.
    /// Rounding would have reversed both answers.
    #[tokio::test]
    async fn the_per_serving_division_floors_exactly_as_the_card_does() {
        let conn = calorie_conn().await;
        assert_eq!(
            ids(&load_corpus(&conn, &ranged(Some(702), Some(702)))
                .await
                .unwrap()),
            vec!["lasagne"],
            "the floored figure is 702"
        );
        assert!(
            ids(&load_corpus(&conn, &ranged(Some(703), Some(703)))
                .await
                .unwrap())
            .is_empty(),
            "703 is what rounding would have given, and it is not the card's number"
        );
    }

    /// A serving the card cannot print is a serving no range can fit. `rub` is 90 kcal
    /// read as feeding a hundred, which floors to 0 — `formatCalories` returns `null`
    /// and the badge shows nothing — so it stays out of a range that would otherwise
    /// hold anything small, rather than being dealt as a card the bound promised
    /// something about and the badge is silent on.
    #[tokio::test]
    async fn a_serving_that_floors_below_one_kcal_is_not_dealt() {
        let conn = calorie_conn().await;
        let corpus = load_corpus(&conn, &ranged(None, Some(500))).await.unwrap();
        assert_eq!(
            ids(&corpus),
            vec!["salad"],
            "the sub-1 serving has no number to fit"
        );
    }

    /// **Either end may be open, and one open end still bounds.** A min alone keeps the
    /// hearty ones; a max alone keeps the light ones; the two are not the same deck, and
    /// neither is the whole corpus.
    ///
    /// This is what "no range set" has to be told apart from: the short-circuit is both
    /// ends `NULL` *together*, so a half-open range that fell through it would deal the
    /// unread recipe and both floors.
    #[tokio::test]
    async fn one_open_end_still_bounds_the_deck() {
        let conn = calorie_conn().await;
        assert_eq!(
            ids(&load_corpus(&conn, &ranged(Some(700), None)).await.unwrap()),
            vec!["feast", "lasagne"],
            "a min alone keeps 702 and 1200 and drops 310"
        );
        assert_eq!(
            ids(&load_corpus(&conn, &ranged(None, Some(700))).await.unwrap()),
            vec!["salad"],
            "a max alone keeps 310 and drops 702 and 1200"
        );
    }

    /// The two ends are not interchangeable: 200–800 and 800–200 are different
    /// questions and only the first has an answer. The lobby cannot send the second
    /// (`session::validate_kcal_range` refuses it), so this pins that the SQL reads `?4`
    /// as the floor and `?5` as the ceiling rather than the other way round.
    #[tokio::test]
    async fn the_two_ends_are_not_interchangeable() {
        let conn = calorie_conn().await;
        assert_eq!(
            ids(&load_corpus(&conn, &ranged(Some(200), Some(800)))
                .await
                .unwrap()),
            vec!["lasagne", "salad"]
        );
        assert!(
            ids(&load_corpus(&conn, &ranged(Some(800), Some(200)))
                .await
                .unwrap())
            .is_empty(),
            "an upside-down range selects nothing rather than the same deck"
        );
    }

    /// The calorie range composes with the bounds already on the plan (#80, #184): each
    /// narrows, none replaces, and a recipe has to satisfy all of them.
    ///
    /// `lasagne` and `salad` both fit the range; only `lasagne` is also read as a dinner
    /// and estimated inside the cap, so it is the one recipe the deck holds.
    #[tokio::test]
    async fn the_calorie_range_composes_with_the_time_cap_and_the_meal() {
        let conn = calorie_conn().await;
        conn.execute(
            "UPDATE recipes SET total_seconds = 1500, sittings = '[\"dinner\"]'
              WHERE id = 'lasagne'",
            (),
        )
        .await
        .unwrap();
        // Fits the range and is read as a dinner, but takes too long.
        conn.execute(
            "UPDATE recipes SET total_seconds = 7200, sittings = '[\"dinner\"]'
              WHERE id = 'salad'",
            (),
        )
        .await
        .unwrap();
        let bounds = Bounds {
            max_total_seconds: Some(1800),
            meal_type: Some(MealType::Dinner),
            min_kcal_per_serving: Some(200),
            max_kcal_per_serving: Some(800),
            ..Bounds::default()
        };
        assert_eq!(
            ids(&load_corpus(&conn, &bounds).await.unwrap()),
            vec!["lasagne"]
        );
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
            ..Bounds::default()
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
    ///
    /// **No seed**, so every test built on it is now the pin on the pre-#220 plan: the
    /// answered row never enters the graph, exactly as #202 shipped it, and the deal
    /// runs on entropy. The seeded plan is the section below.
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

    /// The same round on a plan minted since #220, so its deal is replayable (#225).
    fn continuing_seeded(meal: MealType, channel: &str, voter: &str, seed: i64) -> Bounds {
        Bounds {
            seed: Some(seed),
            ..continuing(meal, channel, voter)
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

    // ---- the two levels the answer is applied at (#202 + #225) --------------

    /// On a **seeded** plan the answered recipe stays in the graph and out of the hand.
    /// Both halves matter and they are asserted separately: in the corpus because that
    /// is what holds the journey still, and out of the deal because #202's guarantee is
    /// unconditional — a card you answered is never handed back.
    #[tokio::test]
    async fn a_seeded_plan_keeps_an_answered_card_in_the_graph_and_out_of_the_hand() {
        let conn = dinner_conn().await;
        answered(&conn, "plan", "stew", "mel", true).await;
        let bounds = continuing_seeded(MealType::Dinner, "plan", "mel", SEED_A);
        let corpus = load_corpus(&conn, &bounds).await.unwrap();

        assert_eq!(
            ids(&corpus),
            vec!["blank", "pancakes", "stew", "uncategorised"],
            "the answered recipe is still a node — removing it would move the journey"
        );
        let dealt: Vec<String> = stops_for(&corpus, &bounds, MAX_LEN)
            .into_iter()
            .map(|s| s.recipe.id)
            .collect();
        assert!(
            !dealt.contains(&"stew".to_owned()),
            "and it is not dealt: {dealt:?}"
        );
        let mut rest = dealt.clone();
        rest.sort();
        assert_eq!(
            rest,
            vec!["blank", "pancakes", "uncategorised"],
            "the rest of the round is dealt as ever"
        );
    }

    /// The same guarantee end to end from the database: answering one card takes that
    /// card out of the hand and leaves every other card exactly where it was. This is
    /// #202's exclusion and #225's replay meeting over real rows rather than a fixture.
    #[tokio::test]
    async fn a_seeded_hand_keeps_its_order_when_a_card_is_answered() {
        let conn = dinner_conn().await;
        let bounds = continuing_seeded(MealType::Dinner, "plan", "mel", SEED_A);
        let before: Vec<String> = stops_for(
            &load_corpus(&conn, &bounds).await.unwrap(),
            &bounds,
            MAX_LEN,
        )
        .into_iter()
        .map(|s| s.recipe.id)
        .collect();
        assert_eq!(before.len(), 4, "the whole dinner round");

        answered(&conn, "plan", &before[1], "mel", false).await;
        let after: Vec<String> = stops_for(
            &load_corpus(&conn, &bounds).await.unwrap(),
            &bounds,
            MAX_LEN,
        )
        .into_iter()
        .map(|s| s.recipe.id)
        .collect();

        let expected: Vec<String> = before
            .iter()
            .filter(|id| **id != before[1])
            .cloned()
            .collect();
        assert_eq!(after, expected, "one card gone, the rest in place");
    }

    /// A plan **older than the seed column** loads exactly what #202 loaded: the
    /// answered row is not in the corpus at all. The mark is for plans that have a
    /// journey to hold still; this one never did.
    #[tokio::test]
    async fn a_plan_with_no_seed_never_loads_the_answered_card() {
        let conn = dinner_conn().await;
        answered(&conn, "plan", "stew", "mel", true).await;
        let corpus = load_corpus(&conn, &continuing(MealType::Dinner, "plan", "mel"))
            .await
            .unwrap();
        assert_eq!(ids(&corpus), vec!["blank", "pancakes", "uncategorised"]);
        assert!(
            corpus.answered.is_empty(),
            "nothing to mark: the row is gone"
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
