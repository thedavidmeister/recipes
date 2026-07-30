//! Where a dish sits in a meal, as far as the corpus actually **states** it (#184).
//!
//! A recipe carries one flat `category` from its source, and that one field mixes
//! two different kinds of claim with a great deal of silence:
//!
//! - `Dessert`, `Side` and `Starter` say what part of a meal the dish is — something
//!   served *with* or *around* the meal rather than as it.
//! - `Breakfast` says which sitting the dish belongs to.
//! - `Beef`, `Chicken`, `Pasta`, `Vegetarian`, `Miscellaneous` and the rest say what
//!   is in the dish or how it is cooked, and **nothing at all** about when it is
//!   eaten.
//!
//! [`course`] reads that field as three answers, and the third is an absence. An
//! absence never *classifies* — see below — but by ruling (#192) a meal round serves
//! only what explicitly matches, so an unread dish is in **no** meal round until the
//! reading lands. Missing data is a scraper/enrichment gap: the fix is reading the
//! corpus, never widening a deck to cover for it.
//!
//! # Silence is not a classification
//!
//! The tempting shortcut is to read "no category said `Dessert`" as "therefore this
//! is a dinner". It is wrong, and it is wrong in the direction that hides itself: it
//! would claim 507 of the corpus's 790 recipes for whichever meal happened to be
//! asked for, on the strength of a source that never said so. `Beef` is not a claim
//! that a dish is dinner; it is a claim about beef. So the only facts acted on here
//! are the ones a source really stated, and everything else stays [`Course::Unstated`]
//! — which narrows nothing.
//!
//! # What the corpus actually carries
//!
//! Measured against production — 790 recipes, 14 categories, none empty:
//!
//! | | | | |
//! |---|---|---|---|
//! | Dessert | 166 | Miscellaneous | 33 |
//! | Vegetarian | 100 | Lamb | 33 |
//! | Beef | 95 | Breakfast | 19 |
//! | Side | 84 | Starter | 14 |
//! | Seafood | 84 | Pasta | 12 |
//! | Chicken | 81 | Vegan | 7 |
//! | Pork | 60 | Goat | 2 |
//!
//! So the corpus states **264 accompaniments** and **one sitting** (`Breakfast`, 19).
//! There is no `Lunch`, `Dinner`, `Snack` or `Drink` category at all. Whether a dish
//! suits breakfast rather than dinner is therefore a question the corpus cannot
//! answer today, and the honest thing is to say so rather than to synthesise an
//! answer out of the silence — see [`Course::Sitting`].
//!
//! The word lists below are the corpus's vocabulary, not an aspiration: a word is
//! added here when a source starts stating it, and an unrecognised word falls to
//! [`Course::Unstated`], which restricts nothing. That direction is deliberate — a
//! source we have not met yet cannot narrow anyone's deck by accident.
//!
//! # The reading that answers what the category cannot (#191)
//!
//! Everything above is about what a **source** stated, and the answer is mostly
//! silence. [`Sitting`] and [`fit`] are the other half: a **reading** of when a dish is
//! actually eaten, produced off the service like the four enrichments before it, and
//! the only thing in this module that can tell breakfast from dinner.
//!
//! The two do not compete. `course` reads a field the source wrote; [`fit`] reads a
//! reading we made. A category of `Breakfast` is still [`Course::Sitting`] and still
//! narrows nothing on its own — deciding that a breakfast dish is not a dinner is a
//! judgement, and judgements belong to the reading, not to a word a source happened to
//! file the dish under.

use serde::{Deserialize, Serialize};

/// Category words that state a dish **accompanies** a meal rather than being one.
///
/// Lowercase, because [`course`] compares against the case-folded category. These
/// three are the corpus's whole vocabulary for it: `Dessert` (166), `Side` (84) and
/// `Starter` (14) of 790.
const ACCOMPANIMENTS: [&str; 3] = ["dessert", "side", "starter"];

/// Category words that state **which sitting** a dish belongs to.
///
/// The corpus states exactly one — `Breakfast`, 19 of 790 — and nothing for lunch,
/// dinner or a snack.
const SITTINGS: [&str; 1] = ["breakfast"];

/// What a recipe's `category` states about the place its dish takes in a meal.
///
/// Three answers, deliberately distinct, because two of them are facts a source
/// stated and the third is silence. Collapsing the third into either of the others is
/// how a corpus that classified 283 of its 790 recipes starts being read as though it
/// had classified all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Course {
    /// The source states the dish is served *with* or *around* a meal and not as one
    /// — `Dessert`, `Side`, `Starter`.
    ///
    /// This is the one exclusion the corpus genuinely supports, and #114's two-tier
    /// vocabulary is the other half of it: dessert and side are *additions*, the
    /// things that come with a meal, never the meal a plan is for. A meal round that
    /// dealt a trifle as the dinner would be contradicting the plan's own question.
    Accompaniment,
    /// The source states the dish belongs to a named sitting, carrying that sitting's
    /// word case-folded (`"breakfast"`).
    ///
    /// **Stated, and deliberately not yet acted on.** A dish the source calls a
    /// breakfast stays in a lunch or a dinner deck, because ruling it out of one
    /// would take the further step of deciding that a breakfast dish is not a dinner
    /// — and no source says that. It is exactly the inference this module refuses to
    /// make from silence, only wearing a label. The variant exists so the claim is
    /// *read* rather than quietly lumped in with `Beef`; acting on it is a question
    /// for a corpus that states more than one sitting.
    Sitting(String),
    /// The category says nothing about where the dish sits in a meal: it names a
    /// protein or a style (`Beef`, `Pasta`, `Vegetarian`, `Miscellaneous`), it is a
    /// word no source has stated before, or there is no category at all.
    ///
    /// **Not "a main."** An absent claim is not a claim, and treating it as one is
    /// the failure this whole module is shaped to avoid.
    Unstated,
}

/// Read one recipe's `category` for what it states about the dish's place in a meal.
///
/// The comparison is case-folded and trimmed because `category` is stored exactly as
/// the source spelled it — nothing normalises it on the way in (unlike equipment,
/// #81) — so `"Dessert"`, `"dessert"` and `" Dessert "` are one word. A `None`
/// category, or one that is blank once trimmed, states nothing, which is
/// [`Course::Unstated`] like any other silence rather than a case of its own.
///
/// This is the whole of the rule, in one place, because more than one feature needs
/// the same answer from opposite directions: #184 keeps a plan's meal round clear of
/// accompaniments, and #147's per-addition rounds want exactly the recipes this
/// refuses. A second implementation of "is this a dessert" would be a second chance
/// to disagree — the reasoning that put [`crate::equipment::capability`] here.
pub fn course(category: Option<&str>) -> Course {
    let Some(word) = category.map(|c| c.trim().to_lowercase()) else {
        return Course::Unstated;
    };
    if ACCOMPANIMENTS.contains(&word.as_str()) {
        return Course::Accompaniment;
    }
    if SITTINGS.contains(&word.as_str()) {
        return Course::Sitting(word);
    }
    Course::Unstated
}

// --- The reading: when a dish is actually eaten (#191) --------------------------

/// One sitting a person eats at — the closed vocabulary a plan is *for* and a reading
/// answers *with*.
///
/// **One type, used from both ends.** A plan's chosen meal (#114) and a recipe's read
/// sittings are the same four words, so they are the same type: the walk's bound is
/// literally `sittings.contains(&meal)`, with nothing in between that could disagree.
/// Two enums with the same four variants and a mapping function would be a second
/// chance to get it wrong — the reasoning that put [`course`] and
/// [`crate::equipment::capability`] in one place each.
///
/// Serde owns the wire form: always the lowercase name, and an unknown or wrongly-cased
/// word is rejected at deserialization, so no reading and no handler ever holds a word
/// outside the vocabulary. The browser sentence-cases for display; the wire, the reading
/// and the database stay lowercase.
///
/// `Ord` is derived, and the declaration order is the canonical one — the order of the
/// day, which is the order [`canonical`] stores a set in and the order a picker shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sitting {
    Breakfast,
    Lunch,
    Dinner,
    Snack,
}

impl Sitting {
    /// Every sitting, in canonical order — the order of the day.
    pub const ALL: [Sitting; 4] = [
        Sitting::Breakfast,
        Sitting::Lunch,
        Sitting::Dinner,
        Sitting::Snack,
    ];

    /// The lowercase canonical form — what the wire carries and the database stores.
    pub fn as_str(self) -> &'static str {
        match self {
            Sitting::Breakfast => "breakfast",
            Sitting::Lunch => "lunch",
            Sitting::Dinner => "dinner",
            Sitting::Snack => "snack",
        }
    }

    /// The inverse of [`Self::as_str`], for reading a stored row back. `None` for
    /// anything outside the vocabulary — the caller decides how loud to be.
    pub fn parse(s: &str) -> Option<Self> {
        Sitting::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// The canonical form of a set of sittings: each at most once, in vocabulary order.
///
/// The reading **means a set** — "lunch or dinner" — so `["dinner","lunch"]` and
/// `["lunch","dinner"]` are one fact and must not be stored as two. Ordering is no part
/// of that meaning, so normalising it is serialising a set rather than repairing a
/// reading; a genuine mistake in the reading (the same sitting twice) is *refused* by
/// [`validate`] instead. Same split, and the same reason, as the chosen-additions list
/// a plan carries (#114).
pub fn canonical(sittings: &[Sitting]) -> Vec<Sitting> {
    Sitting::ALL
        .into_iter()
        .filter(|s| sittings.contains(s))
        .collect()
}

/// Is this a usable reading of when a dish is eaten?
///
/// Two rejections, and the first is the ruling that matters:
///
/// **An empty set is a failed reading, never a fact about the food.** Every dish is
/// eaten at some time. A reading that names no sitting has not discovered a dish nobody
/// eats; it has failed, and storing it would drop the recipe out of the queue for good
/// while leaving the deck no better off. That is the call #158 made on step durations
/// and #162 made on servings, and the call #81 made on an empty equipment reading — a
/// salad still needs a bowl.
///
/// **A repeat is refused rather than deduplicated**, because a set that names `dinner`
/// twice is a reading that did not know what it was asked for, and quietly folding it
/// away is how you never find that out (#81's reasoning, and #74's).
///
/// A word outside the vocabulary needs no check here at all: [`Sitting`] is a closed
/// enum and serde refuses one at the wire, so an unknown word can never reach a reading
/// to be validated.
pub fn validate(sittings: &[Sitting]) -> Result<(), String> {
    if sittings.is_empty() {
        return Err(
            "no sittings — every dish is eaten at some time, so an empty set is a failed \
             reading rather than a fact about the food"
                .into(),
        );
    }
    for (i, sitting) in sittings.iter().enumerate() {
        if sittings[..i].contains(sitting) {
            return Err(format!("sitting {i} repeats {:?}", sitting.as_str()));
        }
    }
    Ok(())
}

/// What a recipe's read sittings say about dealing it to a plan for `meal`.
///
/// Three answers, and the third is an absence — the same shape and the same ruling as
/// [`crate::equipment::Capability`] and [`Course`]. Collapsing [`MealFit::Unread`] into
/// either of the other two is how a corpus nobody has read yet starts being described
/// as though it had been.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MealFit {
    /// The reading names this sitting. The dish is eaten then, so it is dealt.
    Suits,
    /// The reading names sittings, and this is not one of them. **This is the whole
    /// point of the reading**: it is the first thing in the corpus that can keep a roast
    /// out of a breakfast, and it does so on something we read rather than on silence.
    Wrong,
    /// The recipe has no reading. **Not "eaten at no sitting"** — an empty reading is
    /// refused on the way in ([`validate`]), so an empty set can only mean unread, and
    /// when a dish is eaten is simply unknown.
    Unread,
}

/// Read one recipe's sittings against the meal a plan is for (#191).
///
/// This is the whole of the rule, in one place, for the reason [`course`] is: #191
/// keeps a plan's round to the meal it asked for, and #147's per-addition rounds will
/// ask the same question about a dessert. A second implementation would be a second
/// chance to disagree.
///
/// # An unread recipe is excluded from meal rounds, by ruling
///
/// [`MealFit::Unread`] says *we have not read this* — a different fact from
/// [`MealFit::Wrong`]'s *read, and not this meal*, and the walk must be able to tell
/// them apart in its reporting even though, by ruling (#192/#193), both are excluded
/// from a meal round: the round serves only what **explicitly** matches the filter.
/// This matches the kitchen bound's treatment of an unread equipment reading, and for
/// the same written reason — containment is a proof, and admitting unread recipes
/// beside it would mix a proof with a guess.
///
/// The cost is accepted and stated: the reading is produced off the service by a
/// worker somebody runs (#59), so until it has read a recipe no meal round deals it —
/// on the day this lands, that is the whole corpus, and every meal round is empty
/// until the `enrich-meal-times` worker runs against production. Running the worker is
/// the act that delivers the feature; the merge is not it. The same rule then keeps
/// holding: ingest adds recipes unread, and each stays out of every meal round until
/// its reading lands — a deck never contains a guess.
///
/// There is no flag, no threshold and no dated act to remember: the rule is per
/// recipe, so each one joins its decks the moment it is read, and the corpus being
/// read IS the rollout.
pub fn fit(sittings: &[Sitting], meal: Sitting) -> MealFit {
    if sittings.is_empty() {
        return MealFit::Unread;
    }
    if sittings.contains(&meal) {
        MealFit::Suits
    } else {
        MealFit::Wrong
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three words the corpus uses for something that comes with a meal, read as
    /// the claim they are — whatever case or padding the source spelled them in.
    #[test]
    fn a_stated_accompaniment_is_read_as_one() {
        for word in [
            "Dessert",
            "dessert",
            "DESSERT",
            " Dessert ",
            "Side",
            "side",
            "Starter",
            "starter",
        ] {
            assert_eq!(
                course(Some(word)),
                Course::Accompaniment,
                "{word:?} states the dish accompanies a meal"
            );
        }
    }

    /// `Breakfast` is a stated sitting, so it is read as one rather than folded in
    /// with the silence — even though nothing acts on it yet.
    #[test]
    fn a_stated_sitting_is_read_as_one() {
        for word in ["Breakfast", "breakfast", " BREAKFAST "] {
            assert_eq!(
                course(Some(word)),
                Course::Sitting("breakfast".to_string()),
                "{word:?} states which sitting the dish belongs to"
            );
        }
    }

    /// The heart of the ruling: a category that names a protein or a style says
    /// nothing about when the dish is eaten, and so does no category at all. None of
    /// this is "a main" — reading it that way is the classification-from-silence this
    /// module exists to refuse.
    #[test]
    fn a_category_that_says_nothing_about_the_meal_is_unstated() {
        for category in [
            Some("Beef"),
            Some("Chicken"),
            Some("Goat"),
            Some("Lamb"),
            Some("Miscellaneous"),
            Some("Pasta"),
            Some("Pork"),
            Some("Seafood"),
            Some("Vegan"),
            Some("Vegetarian"),
            // A word no source has stated. Unrecognised falls to silence, never to a
            // guess — a new source cannot narrow a deck by surprise.
            Some("Brunch"),
            Some("Appetizer"),
            // Nothing to read at all.
            Some(""),
            Some("   "),
            None,
        ] {
            assert_eq!(
                course(category),
                Course::Unstated,
                "{category:?} states nothing about where the dish sits in a meal"
            );
        }
    }

    /// The whole production vocabulary, verdict by verdict, so the measurement this
    /// module is designed around is pinned rather than remembered. 264 of 790 are
    /// stated accompaniments, 19 state a sitting, and the remaining 507 state
    /// nothing — which is why "not a dessert therefore a dinner" would have been a
    /// claim about two thirds of the corpus.
    #[test]
    fn the_whole_production_vocabulary_is_classified() {
        let corpus = [
            ("Dessert", 166, Course::Accompaniment),
            ("Side", 84, Course::Accompaniment),
            ("Starter", 14, Course::Accompaniment),
            ("Breakfast", 19, Course::Sitting("breakfast".to_string())),
            ("Vegetarian", 100, Course::Unstated),
            ("Beef", 95, Course::Unstated),
            ("Seafood", 84, Course::Unstated),
            ("Chicken", 81, Course::Unstated),
            ("Pork", 60, Course::Unstated),
            ("Miscellaneous", 33, Course::Unstated),
            ("Lamb", 33, Course::Unstated),
            ("Pasta", 12, Course::Unstated),
            ("Vegan", 7, Course::Unstated),
            ("Goat", 2, Course::Unstated),
        ];
        assert_eq!(corpus.len(), 14, "every category production carries");
        assert_eq!(
            corpus.iter().map(|(_, n, _)| n).sum::<usize>(),
            790,
            "every recipe production holds"
        );

        let total = |verdict: &Course| -> usize {
            corpus
                .iter()
                .filter(|(_, _, c)| c == verdict)
                .map(|(_, n, _)| n)
                .sum()
        };
        for (word, _, expected) in &corpus {
            assert_eq!(course(Some(word)), *expected, "{word}");
        }
        assert_eq!(total(&Course::Accompaniment), 264, "stated accompaniments");
        assert_eq!(total(&Course::Unstated), 507, "stated nothing at all");
        assert_eq!(
            total(&Course::Sitting("breakfast".to_string())),
            19,
            "the only sitting the corpus states"
        );
    }

    // --- The reading (#191) ----------------------------------------------------

    use Sitting::{Breakfast, Dinner, Lunch, Snack};

    /// The wire form is the lowercase word and nothing else, both ways — the reading,
    /// the plan's stored `meal_type` and the browser all speak it, so a drift here would
    /// be a drift between all three.
    #[test]
    fn a_sitting_is_its_lowercase_word_on_the_wire() {
        for sitting in Sitting::ALL {
            let json = serde_json::to_string(&sitting).unwrap();
            assert_eq!(json, format!("\"{}\"", sitting.as_str()));
            assert_eq!(
                serde_json::from_str::<Sitting>(&json).unwrap(),
                sitting,
                "round trip"
            );
            assert_eq!(Sitting::parse(sitting.as_str()), Some(sitting));
        }
        // A closed vocabulary is closed at the wire, so no reading can carry a word
        // outside it and no later check has to look for one.
        for outside in ["\"Dinner\"", "\"brunch\"", "\"supper\"", "\"\""] {
            assert!(
                serde_json::from_str::<Sitting>(outside).is_err(),
                "{outside} is not a sitting"
            );
        }
        assert_eq!(Sitting::parse("Dinner"), None, "and not out of band either");
        assert_eq!(Sitting::parse("brunch"), None);
    }

    /// **The ruling this reading turns on.** An empty set is a failed reading, never a
    /// dish nobody eats — the call #158 made on durations and #162 on servings. If this
    /// were accepted the recipe would leave the queue permanently with a set that can
    /// match no plan at all.
    #[test]
    fn an_empty_reading_is_refused_because_every_dish_is_eaten_sometime() {
        let err = validate(&[]).unwrap_err();
        assert!(err.contains("every dish is eaten at some time"), "{err}");
    }

    /// A repeat is a reading that misunderstood the question, so it is refused rather
    /// than quietly folded away — #81's rule about a reading that has to be fixed on the
    /// way in.
    #[test]
    fn a_reading_cannot_name_the_same_sitting_twice() {
        let err = validate(&[Dinner, Lunch, Dinner]).unwrap_err();
        assert!(err.contains("repeats"), "{err}");
        assert!(validate(&[Dinner, Lunch]).is_ok());
    }

    /// Every set the vocabulary can express is a legitimate reading, single or several.
    /// A dish that genuinely suits every sitting is not an error — it is unusual, and
    /// the skill is where that is discouraged, not the validator, because refusing it
    /// here would be inventing a fact about food.
    #[test]
    fn any_non_empty_set_is_a_reading() {
        assert!(validate(&[Breakfast]).is_ok(), "a pancake");
        assert!(validate(&[Lunch, Dinner]).is_ok(), "a chicken curry");
        assert!(validate(&[Breakfast, Snack]).is_ok(), "toast");
        assert!(validate(&Sitting::ALL).is_ok(), "an omelette, at a push");
    }

    /// A set has no order, so two spellings of one fact store as one row. The order is
    /// the order of the day, not the order the model happened to answer in.
    #[test]
    fn a_set_is_stored_in_one_canonical_order() {
        assert_eq!(canonical(&[Dinner, Lunch]), vec![Lunch, Dinner]);
        assert_eq!(canonical(&[Lunch, Dinner]), vec![Lunch, Dinner]);
        assert_eq!(canonical(&[Snack, Breakfast]), vec![Breakfast, Snack]);
        assert_eq!(canonical(&Sitting::ALL), Sitting::ALL.to_vec());
        assert_eq!(canonical(&[]), Vec::<Sitting>::new());
    }

    /// **What the whole issue is for.** A read dish is dealt to the sittings it names
    /// and refused by the ones it does not — the first thing in this module that can
    /// tell breakfast from dinner, and it does it on a reading rather than on silence.
    #[test]
    fn a_read_dish_suits_the_sittings_it_names_and_no_others() {
        // A chicken curry: lunch or dinner, not breakfast.
        let curry = [Lunch, Dinner];
        assert_eq!(fit(&curry, Lunch), MealFit::Suits);
        assert_eq!(fit(&curry, Dinner), MealFit::Suits);
        assert_eq!(fit(&curry, Breakfast), MealFit::Wrong);
        assert_eq!(fit(&curry, Snack), MealFit::Wrong);

        // A roast: dinner alone.
        for meal in [Breakfast, Lunch, Snack] {
            assert_eq!(fit(&[Dinner], meal), MealFit::Wrong, "{meal:?}");
        }
        assert_eq!(fit(&[Dinner], Dinner), MealFit::Suits);
    }

    /// An unread recipe answers **`Unread`** for every meal — a different fact from
    /// `Wrong`, and the walk needs the distinction for its reporting even though, by
    /// ruling (#192), both are excluded from a meal round: only an explicit match is
    /// served, and a gap in our reading is fixed by reading, never by dealing a guess.
    #[test]
    fn an_unread_recipe_is_unread_for_every_meal() {
        for meal in Sitting::ALL {
            assert_eq!(fit(&[], meal), MealFit::Unread, "{meal:?}");
        }
    }
}
