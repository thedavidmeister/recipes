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
//! [`course`] reads that field as three answers, and the third is an absence — the
//! same shape, and the same ruling, as [`crate::equipment::Capability::Unread`]. A
//! dish the corpus says nothing about is **not restricted**, never excluded.
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
}
