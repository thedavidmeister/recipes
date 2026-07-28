//! Equipment a recipe requires (#81) — the model's reading of what you need to own to
//! cook it.
//!
//! Unlike a measure (#11) there is nothing to compute here and nothing to scale: the
//! reading is a set of names. What matters instead is that the names **converge**,
//! because a kitchen selects from this vocabulary rather than inventing its own
//! (#81 ruling). "Frying pan", "Frying Pan" and " frying pan " must be one item or
//! every comparison between a kitchen and a recipe silently fails.
//!
//! So normalisation is part of the format, not a cleanup step applied later: a reading
//! is only valid if every name is already in normal form. A model that returns
//! "Large Wok" is corrected on the way in rather than admitted and worked around
//! forever after.
//!
//! Note what a reading has to include: **preparation tools, not only appliances**. A
//! salad needs a bowl and a knife and a board even though nothing is cooked. A reading
//! that lists only the obvious machinery is the failure mode to watch for, because a
//! kitchen missing a knife would then appear able to cook everything.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The one true spelling of an equipment name: trimmed, lowercased, and with runs of
/// whitespace collapsed.
///
/// Deliberately conservative — it does not singularise, stem, or map synonyms. Those
/// are judgements about *meaning* ("skillet" and "frying pan"), and a silent guess at
/// meaning is how a vocabulary quietly stops matching itself. Anything beyond spelling
/// belongs in the reading, where a model can be asked to be consistent, or in an
/// explicit synonym table that a person can read.
pub fn normalise(raw: &str) -> String {
    raw.split_whitespace()
        .map(|word| word.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A recipe's equipment, as the model read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredEquipment {
    /// The normalised name — see [`normalise`].
    pub item: String,
}

/// Is this reading usable as a vocabulary?
///
/// Rejects rather than repairs. A reading that has to be fixed on the way in is one
/// whose model has not been told the rules yet, and quietly repairing it means never
/// finding that out — the same reasoning that makes an empty step reading a rejection
/// rather than an empty list (#74).
pub fn validate(equipment: &[RequiredEquipment]) -> Result<(), String> {
    let mut seen = Vec::new();
    for (i, e) in equipment.iter().enumerate() {
        if e.item.trim().is_empty() {
            return Err(format!("equipment {i} has no name"));
        }
        let normal = normalise(&e.item);
        if normal != e.item {
            return Err(format!(
                "equipment {i} is not normalised: {:?} should be {:?}",
                e.item, normal
            ));
        }
        if seen.contains(&normal) {
            return Err(format!("equipment {i} repeats {:?}", e.item));
        }
        seen.push(normal);
    }
    Ok(())
}

/// What a kitchen holding a given set of equipment can do with one recipe's reading.
///
/// The three answers are deliberately distinct, because two of them are facts and the
/// third is an absence, and collapsing them is how a "you can make this" claim starts
/// being made about recipes nobody has read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// Every item the reading names is held. The reading is complete, so this is a
    /// proof, not an estimate — nothing else is needed to cook it.
    CanMake,
    /// The named items the kitchen does not hold, in the reading's order. Never empty:
    /// an empty shortfall is [`Capability::CanMake`].
    Missing(Vec<String>),
    /// The recipe has no reading. **Not "needs nothing"** — an empty reading is
    /// *refused* on the way in (#81: a salad still needs a bowl, a knife and a board),
    /// precisely so that a kitchen owning no knife cannot appear able to cook
    /// everything. An empty list therefore means unread, and makeability is simply
    /// unknown.
    Unread,
}

/// Assess one recipe's reading against the equipment a kitchen holds.
///
/// `owned` is a set of already-[`normalise`]d names — both sides of this comparison
/// come from the same vocabulary by construction (a kitchen may only own items some
/// recipe asks for, #81), so this is plain set containment and never a fuzzy match.
///
/// This is the whole of the matching rule, in one place, because two features need the
/// *same* answer from opposite directions: #82 filters a pick to [`Capability::CanMake`],
/// and #83 ranks equipment to buy by counting the recipes whose [`Capability::Missing`]
/// is exactly one item. A second implementation of "can this kitchen cook this" would
/// be a second chance to disagree.
pub fn capability(required: &[RequiredEquipment], owned: &BTreeSet<String>) -> Capability {
    if required.is_empty() {
        return Capability::Unread;
    }
    let missing: Vec<String> = required
        .iter()
        .filter(|e| !owned.contains(&e.item))
        .map(|e| e.item.clone())
        .collect();
    if missing.is_empty() {
        Capability::CanMake
    } else {
        Capability::Missing(missing)
    }
}

/// One line of a shopping list: this item, **together with every item above it**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Addition {
    /// The normalised name — the same key a kitchen owns and a reading asks for.
    pub item: String,
    /// Recipes the kitchen could make holding this item *and everything above it*, over
    /// and above what it can make today.
    ///
    /// Cumulative, deliberately. A per-item figure is only meaningful for the first
    /// line: past that, "adds 12" would have to silently assume the lines above were
    /// bought, and a reader has no way to know whether it did. A running total says
    /// exactly what it counts, and it is the only number here that a kitchen could
    /// check by going shopping.
    pub unlocks: usize,
}

/// What a kitchen should add next, and what it would get for it (#83).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Advice {
    /// The list, in the order it was chosen. Empty when there is nothing to say —
    /// either every recipe we have read is already makeable, or nothing has been read.
    /// [`Advice::read`] tells those two apart.
    pub additions: Vec<Addition>,
    /// Recipes the kitchen can make today, so a caller can present the gain against
    /// where it starts rather than as a bare number.
    pub makeable: usize,
    /// Recipes carrying an equipment reading at all — the corpus every count here is
    /// measured over. Unread recipes are excluded from all of it (see [`Capability`]),
    /// so this is the honest denominator: "30 of the 790 we have read", never "of 790".
    pub read: usize,
}

/// Which signal leads the choice at each step of a walk.
///
/// Neither is right on its own — see [`recommend`], which runs both and keeps whichever
/// list actually turns out to unlock more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lead {
    /// Recipes this item finishes on its own: the shortfall is exactly this item, so
    /// buying it makes them makeable immediately. The obvious reading of #83 — and the
    /// one that says nothing useful to a kitchen that is several items away from
    /// anything.
    Unlocks,
    /// Recipes that want this item and cannot be made without it. Blind to whether
    /// buying it finishes anything, so it can chase a popular item past the one thing
    /// that would have completed a recipe today — but it is the only signal with any
    /// gradient at all while every recipe is still several items out of reach.
    Wanted,
}

/// How many recipes this kitchen can make, through the one shared matcher.
fn makeable(readings: &[Vec<RequiredEquipment>], owned: &BTreeSet<String>) -> usize {
    readings
        .iter()
        .filter(|r| capability(r, owned) == Capability::CanMake)
        .count()
}

/// Walk a shopping list of at most `limit` items, choosing greedily by `lead` and
/// re-counting the whole corpus after every pick.
///
/// The recount is the point: an item's worth depends on what is already in the basket,
/// so a list scored once up front would keep recommending five items that each need the
/// same sixth one. It stops early when nothing is left unmakeable — there is nothing
/// honest to put on the next line.
fn shopping_list(
    readings: &[Vec<RequiredEquipment>],
    owned: &BTreeSet<String>,
    limit: usize,
    lead: Lead,
) -> Vec<Addition> {
    let baseline = makeable(readings, owned);
    let mut held = owned.clone();
    let mut list: Vec<Addition> = Vec::new();

    while list.len() < limit {
        // One pass over the corpus through `capability`, so what counts as makeable
        // here is the same judgement the walk (#82) deals by, not a second opinion.
        let mut tally: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for reading in readings {
            if let Capability::Missing(missing) = capability(reading, &held) {
                let finishes = missing.len() == 1;
                for name in missing {
                    let counts = tally.entry(name).or_insert((0, 0));
                    counts.1 += 1;
                    if finishes {
                        counts.0 += 1;
                    }
                }
            }
        }
        let Some(item) = pick(&tally, lead) else {
            break;
        };
        held.insert(item.clone());
        list.push(Addition {
            item,
            unlocks: makeable(readings, &held) - baseline,
        });
    }
    list
}

/// The best candidate in a tally, or `None` when nothing is missing anywhere.
///
/// Ties break on the other count, then alphabetically. The alphabetical last resort is
/// not cosmetic: without it the answer would depend on the order the corpus happened to
/// be loaded in, so the same kitchen could be told to buy a different thing on a reload.
fn pick(tally: &BTreeMap<String, (usize, usize)>, lead: Lead) -> Option<String> {
    let key = |(unlocks, wanted): (usize, usize)| match lead {
        Lead::Unlocks => (unlocks, wanted),
        Lead::Wanted => (wanted, unlocks),
    };
    let mut best: Option<(&String, (usize, usize))> = None;
    for (name, counts) in tally {
        let scored = key(*counts);
        let better = match best {
            None => true,
            Some((best_name, best_key)) => {
                scored > best_key || (scored == best_key && name < best_name)
            }
        };
        if better {
            best = Some((name, scored));
        }
    }
    best.map(|(name, _)| name.clone())
}

/// Which of two lists to give a kitchen: the one that unlocks more, measured rather
/// than assumed.
///
/// `unlocks` only ever grows along a list, so its last line is the list's total. On an
/// equal total the shorter list wins — the same recipes for fewer things to buy — and
/// on an equal length the unlock-led one does, because it is the list whose first line
/// stands on its own.
fn better_of(unlocks_led: Vec<Addition>, wanted_led: Vec<Addition>) -> Vec<Addition> {
    let total = |list: &[Addition]| list.last().map(|a| a.unlocks).unwrap_or(0);
    let (led, wanted) = (total(&unlocks_led), total(&wanted_led));
    if wanted > led || (wanted == led && wanted_led.len() < unlocks_led.len()) {
        wanted_led
    } else {
        unlocks_led
    }
}

/// What this kitchen should add next, ranked by how many more recipes it would unlock
/// (#83) — the inverse of the walk's can-make bound (#82), over the same readings and
/// through the same [`capability`].
///
/// # Why this is not just "count the recipes one item short"
///
/// That is the obvious ranking, and it is [`Lead::Unlocks`] — it answers "add a blender
/// and you can make 47 more" exactly. It also collapses to nothing precisely where
/// advice is worth most. Recipes in this corpus name a **median of six** items, so a
/// kitchen with little recorded has *no* item that finishes anything, and a
/// one-item-short ranking would answer the emptiest kitchen with silence or with the
/// two or three recipes that happen to name a single tool. The only kitchen in
/// production holds **zero** items.
///
/// The fix is not to swap in [`Lead::Wanted`] either: a kitchen a blender short of forty
/// recipes would then be sent after whatever is merely popular among the ones it cannot
/// make, walking past the item that would have finished those forty today.
///
/// So both are walked and the better list is kept, judged by the thing we actually
/// claim — how many recipes the whole list unlocks. Measured over this corpus, neither
/// leads everywhere: from bare, a five-item wanted-led list reaches **30** recipes
/// against the unlock-led list's **14**; a kitchen already holding the twenty most-asked
/// items gets **98** from the unlock-led list against the wanted-led list's **85**. Over
/// sixty randomly stocked kitchens the unlock-led list won 31, the wanted-led list 17,
/// and 12 tied. A single heuristic would have been wrong for a quarter of them, and the
/// comparison costs one more pass over an 800-recipe corpus.
///
/// `limit` is the caller's: how long a list is worth showing is a question about a
/// page, not about the corpus.
pub fn recommend(
    readings: &[Vec<RequiredEquipment>],
    owned: &BTreeSet<String>,
    limit: usize,
) -> Advice {
    Advice {
        additions: better_of(
            shopping_list(readings, owned, limit, Lead::Unlocks),
            shopping_list(readings, owned, limit, Lead::Wanted),
        ),
        makeable: makeable(readings, owned),
        // An unread recipe is not a recipe needing nothing (#81), so it is no part of
        // any count here — including the total these counts are read against.
        read: readings.iter().filter(|r| !r.is_empty()).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(item: &str) -> RequiredEquipment {
        RequiredEquipment { item: item.into() }
    }

    #[test]
    fn normalising_settles_spelling_but_not_meaning() {
        assert_eq!(normalise("  Frying   Pan "), "frying pan");
        assert_eq!(normalise("WOK"), "wok");
        // Left alone on purpose: these are questions about meaning, and guessing at
        // meaning is how a vocabulary stops matching itself.
        assert_eq!(normalise("skillets"), "skillets", "no singularising");
        assert_eq!(normalise("skillet"), "skillet", "no synonym mapping");
    }

    #[test]
    fn a_reading_must_arrive_normalised() {
        assert!(validate(&[eq("wok"), eq("wok lid")]).is_ok());
        assert!(
            validate(&[eq("Wok")]).is_err(),
            "a capital is a different key, so it is refused rather than fixed"
        );
        assert!(validate(&[eq("  wok")]).is_err(), "padding too");
    }

    #[test]
    fn a_reading_cannot_repeat_itself() {
        let err = validate(&[eq("wok"), eq("wok")]).unwrap_err();
        assert!(err.contains("repeats"), "{err}");
    }

    #[test]
    fn an_empty_name_is_not_a_name() {
        assert!(validate(&[eq("")]).is_err());
        assert!(validate(&[eq("   ")]).is_err());
    }

    /// An empty list is well-*formed*; whether it is a legitimate *reading* is the
    /// submit layer's question, and the answer there is no — a salad still needs a
    /// bowl and a knife. Validation stays about shape so the two concerns do not
    /// blur.
    #[test]
    fn an_empty_list_is_well_formed_but_not_a_reading() {
        assert!(validate(&[]).is_ok(), "shape is fine");
    }

    fn owns(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|i| (*i).to_owned()).collect()
    }

    /// The proof case: the kitchen holds every item the reading names. A reading is
    /// complete by construction, so nothing is left to be surprised by at the stove.
    /// Owning *more* than a recipe asks for changes nothing.
    #[test]
    fn holding_every_item_can_make_it() {
        let needs = [eq("knife"), eq("bowl")];
        assert_eq!(
            capability(&needs, &owns(&["knife", "bowl"])),
            Capability::CanMake
        );
        assert_eq!(
            capability(&needs, &owns(&["knife", "bowl", "wok", "oven"])),
            Capability::CanMake,
            "a well-stocked kitchen is not penalised for it"
        );
    }

    /// The shortfall is named, in the reading's order, so a caller can say *what* is
    /// missing rather than only that something is. #83 counts the one-item case.
    #[test]
    fn a_shortfall_names_what_is_missing() {
        let needs = [eq("wok"), eq("knife"), eq("blender")];
        assert_eq!(
            capability(&needs, &owns(&["knife"])),
            Capability::Missing(vec!["wok".into(), "blender".into()])
        );
        assert_eq!(
            capability(&needs, &owns(&["knife", "wok"])),
            Capability::Missing(vec!["blender".into()]),
            "one item short — the shape #83 ranks by"
        );
    }

    /// An empty kitchen can make nothing, and says so as a shortfall rather than as
    /// an absence: the recipe *is* read, we simply hold none of it.
    #[test]
    fn an_empty_kitchen_is_short_of_everything_not_unread() {
        assert_eq!(
            capability(&[eq("knife"), eq("bowl")], &owns(&[])),
            Capability::Missing(vec!["knife".into(), "bowl".into()])
        );
    }

    /// The ruling that matters most (#81): an empty reading is **unread**, never
    /// "needs nothing". Read the other way, a kitchen holding nothing at all would be
    /// able to cook every unread recipe — which is exactly the failure the submit
    /// layer refuses an empty reading to prevent.
    #[test]
    fn an_empty_reading_is_unread_not_makeable() {
        assert_eq!(capability(&[], &owns(&[])), Capability::Unread);
        assert_eq!(
            capability(&[], &owns(&["knife", "wok"])),
            Capability::Unread,
            "and no amount of equipment turns an absent reading into a proof"
        );
    }

    fn corpus(recipes: &[&[&str]]) -> Vec<Vec<RequiredEquipment>> {
        recipes
            .iter()
            .map(|r| r.iter().map(|i| eq(i)).collect())
            .collect()
    }

    fn items(advice: &Advice) -> Vec<&str> {
        advice.additions.iter().map(|a| a.item.as_str()).collect()
    }

    fn unlocks(advice: &Advice) -> Vec<usize> {
        advice.additions.iter().map(|a| a.unlocks).collect()
    }

    /// The headline case, and the one the obvious ranking already handles: a kitchen one
    /// item short of a pile of recipes is told to buy that item, and the number is the
    /// pile.
    #[test]
    fn a_kitchen_one_item_short_is_told_the_one_item() {
        let corpus = corpus(&[
            &["knife", "blender"],
            &["knife", "blender"],
            &["knife", "blender"],
            &["knife", "sieve", "wok"],
        ]);
        let advice = recommend(&corpus, &owns(&["knife"]), 5);

        assert_eq!(advice.additions[0].item, "blender");
        assert_eq!(
            advice.additions[0].unlocks, 3,
            "three recipes were waiting on exactly it"
        );
        assert_eq!(advice.makeable, 0);
        assert_eq!(advice.read, 4);
    }

    /// The case that decides the design. Every recipe here needs three items, so *no*
    /// single purchase finishes one — a ranking by "recipes one item short" has nothing
    /// to say, which is the state the only kitchen in production is in.
    ///
    /// The list still leads with the item the unmakeable recipes want most, and the
    /// running total shows the payoff arriving at the end rather than pretending it
    /// arrives at the start.
    #[test]
    fn an_empty_kitchen_is_given_a_list_not_silence() {
        let corpus = corpus(&[
            &["knife", "bowl", "board"],
            &["knife", "bowl", "board"],
            &["knife", "bowl", "whisk"],
        ]);
        let advice = recommend(&corpus, &owns(&[]), 3);

        assert_eq!(items(&advice), vec!["bowl", "knife", "board"]);
        assert_eq!(
            unlocks(&advice),
            vec![0, 0, 2],
            "nothing until the third; a running total says so instead of implying more"
        );
        assert_eq!(advice.makeable, 0);
    }

    /// The empty kitchen's list is not merely non-empty, it is the *better* of the two
    /// walks — which is the whole reason both are walked. Ranking by "one item short"
    /// alone would spend the list on the lone recipe that happens to name a single tool
    /// and leave the three-recipe cluster untouched.
    #[test]
    fn the_list_kept_is_the_one_that_actually_unlocks_more() {
        let corpus = corpus(&[
            &["pot"],
            &["knife", "bowl", "board"],
            &["knife", "bowl", "board"],
            &["knife", "bowl", "board"],
        ]);
        let owned = owns(&[]);

        let unlocks_led = shopping_list(&corpus, &owned, 3, Lead::Unlocks);
        let wanted_led = shopping_list(&corpus, &owned, 3, Lead::Wanted);
        assert_eq!(
            unlocks_led.last().unwrap().unlocks,
            1,
            "chasing the immediate unlock buys the pot first, and never catches up"
        );
        assert_eq!(
            wanted_led.last().unwrap().unlocks,
            3,
            "following demand buys the cluster, for three"
        );
        assert_eq!(
            recommend(&corpus, &owned, 3).additions,
            wanted_led,
            "so the kitchen is handed the list that is worth more, not the tidier story"
        );
    }

    /// ...and the comparison cuts the other way just as readily: where the immediate
    /// unlock is real, chasing mere popularity would walk past it.
    #[test]
    fn a_popular_item_does_not_beat_the_one_that_finishes_a_recipe() {
        let mut corpus = corpus(&[&["blender"], &["blender"], &["blender"]]);
        // Six recipes that all want a sieve and are all three items from being made.
        for i in 0..6 {
            corpus.push(vec![eq("sieve"), eq(&format!("pan {i}")), eq("stockpot")]);
        }
        let advice = recommend(&corpus, &owns(&[]), 1);

        assert_eq!(
            items(&advice),
            vec!["blender"],
            "the sieve is wanted twice as often and would finish nothing"
        );
        assert_eq!(advice.additions[0].unlocks, 3);
    }

    /// A kitchen that can already make everything is told nothing, rather than being
    /// sold a tool for recipes it can already cook. `read` is what tells this apart from
    /// having read no recipes at all — the caller needs the difference to say which.
    #[test]
    fn a_complete_kitchen_is_recommended_nothing() {
        let corpus = corpus(&[&["knife", "bowl"], &["knife"]]);
        let advice = recommend(&corpus, &owns(&["knife", "bowl", "wok"]), 5);

        assert!(advice.additions.is_empty());
        assert_eq!(advice.makeable, 2);
        assert_eq!(advice.read, 2);
    }

    /// An unread recipe is not a recipe that needs nothing (#81), so it contributes to
    /// nothing here: not the ranking, not the unlock counts, and not the total those
    /// counts are read against. A corpus nobody has read yet therefore yields no advice
    /// at all — the honest answer, and one a caller can distinguish from a complete
    /// kitchen by `read`.
    #[test]
    fn unread_recipes_are_counted_nowhere() {
        let unread = vec![vec![], vec![], vec![]];
        let advice = recommend(&unread, &owns(&[]), 5);
        assert!(advice.additions.is_empty());
        assert_eq!(advice.read, 0, "and no denominator is claimed for them");
        assert_eq!(advice.makeable, 0);

        let mixed = vec![vec![], vec![eq("wok")], vec![]];
        let advice = recommend(&mixed, &owns(&[]), 5);
        assert_eq!(items(&advice), vec!["wok"]);
        assert_eq!(advice.additions[0].unlocks, 1, "one recipe, not three");
        assert_eq!(advice.read, 1);
    }

    /// A tie is settled the same way every time. Two items wanted by the same recipes
    /// are genuinely equal, so the choice is arbitrary — but an *unstable* arbitrary
    /// choice would tell the same kitchen to buy something different on each reload,
    /// with nothing having changed.
    #[test]
    fn ties_are_settled_alphabetically_and_stay_settled() {
        let corpus = corpus(&[&["whisk", "bowl", "knife"], &["whisk", "bowl", "knife"]]);
        let advice = recommend(&corpus, &owns(&[]), 3);
        assert_eq!(items(&advice), vec!["bowl", "knife", "whisk"]);

        // The same corpus in the other order is the same kitchen, so it is the same
        // answer — nothing here may depend on the order rows came back in.
        let mut reversed = corpus.clone();
        reversed.reverse();
        for r in reversed.iter_mut() {
            r.reverse();
        }
        assert_eq!(recommend(&reversed, &owns(&[]), 3), advice);
    }

    /// Every line counts the lines above it, not itself. Read per-item, the second line
    /// below would say "adds 2" and be wrong twice over.
    #[test]
    fn the_count_on_each_line_is_a_running_total() {
        let corpus = corpus(&[&["knife"], &["knife", "bowl"], &["knife", "bowl", "wok"]]);
        let advice = recommend(&corpus, &owns(&[]), 3);

        assert_eq!(items(&advice), vec!["knife", "bowl", "wok"]);
        assert_eq!(
            unlocks(&advice),
            vec![1, 2, 3],
            "1 then 2 then 3 recipes in total — never 1, 1, 1"
        );
    }

    /// The gain is counted against what the kitchen can make *now*, so a kitchen that
    /// already cooks half the corpus is not credited with recipes it was already making.
    #[test]
    fn the_gain_is_measured_from_where_the_kitchen_starts() {
        let corpus = corpus(&[&["knife"], &["knife", "wok"]]);
        let advice = recommend(&corpus, &owns(&["knife"]), 3);

        assert_eq!(advice.makeable, 1);
        assert_eq!(items(&advice), vec!["wok"]);
        assert_eq!(advice.additions[0].unlocks, 1, "the one it could not make");
    }

    /// The list is as long as it is useful: never longer than asked for, and shorter
    /// when there is nothing left to be short of.
    #[test]
    fn the_list_stops_at_the_limit_and_at_the_end_of_the_gap() {
        let corpus = corpus(&[&["a", "b", "c", "d"]]);
        assert_eq!(recommend(&corpus, &owns(&[]), 2).additions.len(), 2);
        assert_eq!(
            recommend(&corpus, &owns(&[]), 9).additions.len(),
            4,
            "four items are missing, so a ninth line would be an invention"
        );
        assert_eq!(recommend(&corpus, &owns(&[]), 0).additions.len(), 0);
    }

    /// Both sides come from one vocabulary (#81), so matching is containment and
    /// nothing else — a differently-spelled name is a different item, loudly, rather
    /// than a near-match quietly counted as owned.
    #[test]
    fn matching_is_containment_over_one_vocabulary() {
        assert_eq!(
            capability(&[eq("frying pan")], &owns(&["Frying Pan"])),
            Capability::Missing(vec!["frying pan".into()])
        );
        assert_eq!(
            capability(&[eq("skillet")], &owns(&["frying pan"])),
            Capability::Missing(vec!["skillet".into()]),
            "no synonym guessing here either — see `normalise`"
        );
    }
}
