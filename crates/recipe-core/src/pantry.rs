//! What a kitchen's pantry already covers on a shopping list (#156).
//!
//! `buy` starts with the things you already own ticked — nobody walks to the shop for
//! salt that is in the cupboard. This module is the whole of that decision: given the
//! names on a recipe's shopping list and the names in a kitchen's pantry, which lines
//! the kitchen already has, and which pantry entry answered for each.
//!
//! It lives here, beside [`crate::equipment::capability`], for the same reason that one
//! does: a second implementation of "does this kitchen have this" would be a second
//! chance to disagree.
//!
//! # Both sides come from one vocabulary
//!
//! A pantry may only hold an ingredient some recipe cooks with — the add endpoint
//! checks the name against [`crate::equipment::normalise`]d readings of the corpus
//! (`backend::enrich::vocabulary`) and refuses anything else. So both sides of every
//! comparison here are names the corpus itself uses, already lowercased, trimmed and
//! whitespace-collapsed. There is no free text to be clever about.
//!
//! # The rule: the whole name, singularised — and nothing else
//!
//! Two names match when they are **the same name**, after singularising each **as one
//! whole string**. `onions` matches `onion`; `potatoes` matches `potato`; `spring
//! onions` matches `spring onion` and **not** `onion`.
//!
//! There is deliberately no head-word match, no tail-word match, no substring match, no
//! stemming and no synonym table. A false pre-tick means somebody does not buy
//! something they needed, which is worse than a missed one, and every looser rule is
//! wrong on this corpus in ways that are not marginal. Measured against the 948 distinct
//! ingredient names the corpus actually holds, a last-word rule produces **364**
//! pantry→line pairs that are simply different foods — `butter` would tick
//! `peanut butter`, `milk` would tick `coconut milk`, `flour` would tick `corn flour`,
//! `onion` would tick `spring onions`, `pepper` would tick all of `black pepper`,
//! `cayenne pepper`, `habanero pepper` and `romano pepper`.
//!
//! **Its failure mode is under-matching, on purpose.** A pantry holding `onion` will
//! not tick a line reading `chopped onion` or `red onions`, and `olive oil` will not
//! tick `extra virgin olive oil` — those are three-quarters of the same thing, and the
//! rule says nothing about them. The cost is buying an onion you had. The cost of the
//! other direction is a dinner with no onion in it.
//!
//! The one thing this rule cannot repair is ambiguity already in the vocabulary itself:
//! the corpus writes `pepper` for both the spice and the capsicum, so a pantry holding
//! `pepper` ticks both. Matching on names inherits that; it does not create it, and no
//! rule that only sees names could resolve it.
//!
//! # Why singularise at all
//!
//! Because the corpus splits one food across both spellings, arbitrarily — whichever
//! the source happened to write. `potato` is **not in the vocabulary at all** (only
//! `potatoes`, in 87 recipes); `carrot` is in 1 recipe against `carrots` in 84; `onion`
//! 224 against `onions` 29; `egg` 118 against `eggs` 106. Without the fold a pantry
//! stocked with the singular names a person reaches for covers 2968 of the corpus's
//! 8160 ingredient lines; with it, 3320.
//!
//! And it is safe *here* because the key space is closed and was audited whole: folding
//! all 948 vocabulary names collides exactly nine groups, and every one is a genuine
//! singular/plural pair of the same food — `carrot(s)`, `onion(s)`, `egg(s)`,
//! `tomato(es)`, `free-range egg(s)`, `lemon(s)`, `clove(s)`, `chicken breast(es)`,
//! `bun(s)`. (`cloves` was the one worth checking: all fifteen corpus lines spelling it
//! are the spice — garlic is always written `garlic clove`.) The eight vocabulary names
//! that end in `s` without being plurals — `hummus`, `couscous`, `fromage frais`,
//! `khus khus`, `asparagus`, `white asparagus`, `lemongrass`, `petit pois` — are all
//! left alone by the guards below.

use std::collections::BTreeMap;

use crate::models::Ingredient;

/// The key two names must share to be the same thing: the normalised name,
/// singularised as **one whole string**.
///
/// Never per word — that is the difference between `spring onions` → `spring onion`
/// (right) and `spring onions` → `onion` (a different vegetable).
///
/// The suffix rules are English and are meant to be dull. Each guard earns its place
/// against the real vocabulary rather than against grammar in general: the length floors
/// keep three-letter words whole, `ss`/`us`/`is` keep the mass nouns the corpus actually
/// contains (`lemongrass`, `couscous`, `petit pois`) from being cut down to a stem that
/// is not a word.
///
/// A name this does not recognise as plural is simply left as it is. Folding is only
/// ever allowed to *join* two spellings of one food; it is never the thing that decides
/// two foods are different.
pub fn key(name: &str) -> String {
    singular(&crate::equipment::normalise(name))
}

fn singular(w: &str) -> String {
    let len = w.chars().count();
    // …ies → …y (blackberries → blackberry). Before the bare -s rule, which would
    // otherwise leave "blackberrie".
    if len > 4 && w.ends_with("ies") {
        return format!("{}y", &w[..w.len() - 3]);
    }
    // …oes → …o (tomatoes → tomato, potatoes → potato).
    if len > 4 && w.ends_with("oes") {
        return w[..w.len() - 2].to_owned();
    }
    // …es after a sibilant → drop the "es" (dishes → dish, boxes → box).
    if len > 4 && w.ends_with("es") {
        let stem = &w[..w.len() - 2];
        if stem.ends_with('s')
            || stem.ends_with('x')
            || stem.ends_with('z')
            || stem.ends_with("ch")
            || stem.ends_with("sh")
        {
            return stem.to_owned();
        }
    }
    // The ordinary plural. `ss`/`us`/`is` are the mass-noun guards.
    if len > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") && !w.ends_with("is")
    {
        return w[..w.len() - 1].to_owned();
    }
    w.to_owned()
}

/// A kitchen's pantry, indexed by [`key`] so a shopping list can be asked about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pantry {
    /// key → the pantry entry as the kitchen stores it, so a pre-tick can say which
    /// jar answered for the line rather than only that something did.
    by_key: BTreeMap<String, String>,
}

impl Pantry {
    /// Index a kitchen's stored pantry entries.
    ///
    /// A pantry that holds both spellings of one food (`onion` **and** `onions` — the
    /// picker offers them as separate entries, because the corpus does) folds to one
    /// key, and the first entry in sorted order is the one named on a pre-tick. Which
    /// one is arbitrary and only affects the words shown; both are the same jar.
    pub fn new<I, S>(items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut by_key: BTreeMap<String, String> = BTreeMap::new();
        let mut items: Vec<String> = items
            .into_iter()
            .map(|i| crate::equipment::normalise(i.as_ref()))
            .filter(|i| !i.is_empty())
            .collect();
        items.sort();
        for item in items {
            by_key.entry(key(&item)).or_insert(item);
        }
        Self { by_key }
    }

    /// Nothing in it. A kitchen with no pantry, which is every kitchen until somebody
    /// stocks one — worth asking before doing any work.
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// The pantry entry that answers for this ingredient name, if any.
    ///
    /// Returns the entry rather than a bool because a pre-tick says *why* it is ticked
    /// ("already in the pantry — salt"), and a bool would make the caller guess.
    pub fn holds(&self, ingredient_name: &str) -> Option<&str> {
        self.by_key.get(&key(ingredient_name)).map(String::as_str)
    }
}

/// The names of the lines a shopping list shows, **in the list's own index order**.
///
/// `buy_checks` is keyed by that index, so this projection is what makes a pre-tick
/// land on the line it is about. It is the structured reading's `item` (#11) — the
/// ingredient's *name*, separated from its measure — and a line with no reading, or a
/// reading with an empty name, is not on the list at all and so takes no index.
///
/// **This must agree, exactly, with `shoppingLines` in `frontend/src/lib/shopping.ts`**,
/// which builds the very list the indices count against: the browser reads the recipe
/// straight from Turso (there is no WASM, deliberately — see CLAUDE.md) while the
/// server holds the ticks, so the same filter is stated in two languages and there is
/// no third place to put it. Both sides are pinned by a test over the same fixture:
/// `drops_unread_lines_so_indices_match_the_shopping_list` here, and
/// `indexes only the lines the checklist shows` in `shopping.test.ts`.
pub fn shopping_names(ingredients: &[Ingredient]) -> Vec<String> {
    ingredients
        .iter()
        .filter_map(|i| i.structured.as_ref())
        .filter(|s| !s.item.trim().is_empty())
        .map(|s| s.item.clone())
        .collect()
}

/// Which lines of a shopping list the kitchen already has: `(index, pantry entry)`.
///
/// `names` is [`shopping_names`]' output, so the indices are the checklist's own.
/// Absent from the result means "buy it" — including every line of an unread recipe,
/// because an unread recipe has no names to match and a pantry can only answer about
/// names. That is the same fail-safe direction as everything else here.
pub fn preticks(names: &[String], pantry: &Pantry) -> Vec<(usize, String)> {
    if pantry.is_empty() {
        return Vec::new();
    }
    names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| pantry.holds(n).map(|item| (i, item.to_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::{Amount, Quantity, StructuredMeasure};

    fn ing(name: &str, reading: Option<&str>) -> Ingredient {
        Ingredient {
            name: name.into(),
            measure: None,
            structured: reading.map(|item| StructuredMeasure {
                item: item.into(),
                amount: Some(Amount::Quantified {
                    quantity: Quantity::Exact { value: 1.0 },
                    unit: None,
                    size: None,
                }),
                preparation: None,
                note: None,
            }),
        }
    }

    fn pantry(items: &[&str]) -> Pantry {
        Pantry::new(items.iter().copied())
    }

    /// The plain case, and the reason the feature exists: the staples. Every name here
    /// is one the corpus actually holds, with the number of recipes that use it.
    #[test]
    fn a_stocked_staple_is_already_had() {
        let p = pantry(&["salt", "olive oil", "black pepper", "water"]);
        assert_eq!(p.holds("salt"), Some("salt")); // 302 recipes
        assert_eq!(p.holds("olive oil"), Some("olive oil")); // 190
        assert_eq!(p.holds("black pepper"), Some("black pepper")); // 77
        assert_eq!(p.holds("water"), Some("water")); // 137
        assert_eq!(p.holds("garlic"), None, "not stocked, so not had");
    }

    /// Spelling is settled before matching, on both sides — a kitchen's entry and a
    /// recipe's reading are both normalised, so case and padding never decide anything.
    #[test]
    fn spelling_is_settled_before_matching() {
        let p = pantry(&["  Olive   OIL "]);
        assert_eq!(p.holds("olive oil"), Some("olive oil"));
        assert_eq!(p.holds("Olive Oil"), Some("olive oil"));
    }

    /// Singular and plural of the same food are one thing. Each pair below is a real
    /// collision group from the corpus's own 948 names, and the counts are why it
    /// matters: `potato` is not in the vocabulary at all, only `potatoes`.
    #[test]
    fn one_food_spelled_two_ways_is_one_food() {
        let p = pantry(&["onion", "egg", "tomato", "potato", "carrot", "lemon", "bun"]);
        assert_eq!(p.holds("onions"), Some("onion")); // 224 vs 29
        assert_eq!(p.holds("eggs"), Some("egg")); // 118 vs 106
        assert_eq!(p.holds("tomatoes"), Some("tomato")); // 47 vs 21
        assert_eq!(p.holds("potatoes"), Some("potato")); // 0 vs 87
        assert_eq!(p.holds("carrots"), Some("carrot")); // 1 vs 84
        assert_eq!(p.holds("lemons"), Some("lemon")); // 58 vs 6
        assert_eq!(p.holds("buns"), Some("bun"));

        // And the other way round — the pantry is whichever spelling the picker offered.
        let p = pantry(&["potatoes", "carrots", "eggs"]);
        assert_eq!(p.holds("potato"), Some("potatoes"));
        assert_eq!(p.holds("carrot"), Some("carrots"));
        assert_eq!(p.holds("egg"), Some("eggs"));
    }

    /// The near misses the rule **refuses**, one per shape, every pair drawn from names
    /// the corpus really holds. Each of these is a different food, and ticking one off
    /// because the other is in the cupboard sends somebody home without it.
    #[test]
    fn a_variety_is_not_the_plain_thing() {
        let p = pantry(&[
            "onion", "butter", "milk", "flour", "pepper", "sugar", "rice", "oil", "salt", "water",
            "chicken",
        ]);
        // The issue's own example, and the reason a last-word rule is out.
        assert_eq!(p.holds("spring onions"), None);
        assert_eq!(p.holds("red onions"), None);
        assert_eq!(p.holds("chopped onion"), None);
        // The rest of the 364 pairs a last-word rule would have got wrong.
        assert_eq!(p.holds("peanut butter"), None);
        assert_eq!(p.holds("coconut milk"), None);
        assert_eq!(p.holds("corn flour"), None);
        assert_eq!(p.holds("cayenne pepper"), None);
        assert_eq!(p.holds("black pepper"), None);
        assert_eq!(p.holds("icing sugar"), None);
        assert_eq!(p.holds("jasmine rice"), None);
        assert_eq!(p.holds("olive oil"), None);
        assert_eq!(p.holds("kosher salt"), None);
        assert_eq!(p.holds("rose water"), None);
        assert_eq!(p.holds("chicken stock"), None);
        // Nor is it a substring match in the other direction.
        assert_eq!(p.holds("buttermilk"), None);
        assert_eq!(p.holds("butternut squash"), None);
    }

    /// And the mirror: holding the *variety* does not cover the plain thing either.
    /// Someone with extra virgin olive oil may genuinely have no plain oil.
    #[test]
    fn the_plain_thing_is_not_a_variety_either() {
        let p = pantry(&["extra virgin olive oil", "spring onions", "peanut butter"]);
        assert_eq!(p.holds("oil"), None);
        assert_eq!(p.holds("olive oil"), None);
        assert_eq!(p.holds("onion"), None);
        assert_eq!(p.holds("butter"), None);
    }

    /// The mass nouns. Every one of these is a real vocabulary name ending in `s` that
    /// is not a plural; cutting the `s` off would invent a word and could only ever
    /// make a wrong match.
    #[test]
    fn a_word_ending_in_s_is_not_always_a_plural() {
        for word in [
            "hummus",
            "couscous",
            "fromage frais",
            "khus khus",
            "asparagus",
            "white asparagus",
            "lemongrass",
            "petit pois",
        ] {
            assert_eq!(key(word), word, "{word} is not a plural");
        }
    }

    /// The fold is applied to the whole name, never inside it. This is the single
    /// property that keeps `spring onions` away from `onion`.
    #[test]
    fn the_fold_is_over_the_whole_name_not_its_words() {
        assert_eq!(key("spring onions"), "spring onion");
        assert_eq!(key("red onions"), "red onion");
        assert_eq!(key("cherry tomatoes"), "cherry tomato");
        assert_ne!(key("spring onions"), key("onion"));
        assert_ne!(key("cherry tomatoes"), key("tomato"));
    }

    /// The suffix rules, stated. `cloves` is the one that needed checking against the
    /// corpus rather than against grammar: all fifteen lines spelling it mean the
    /// spice, and garlic is always written `garlic clove`.
    #[test]
    fn plural_suffixes_fold_and_short_words_do_not() {
        assert_eq!(key("blackberries"), "blackberry");
        assert_eq!(key("potatoes"), "potato");
        assert_eq!(key("cloves"), "clove");
        assert_eq!(key("chicken breasts"), "chicken breast");
        assert_eq!(key("free-range eggs"), "free-range egg");
        // Too short to be a plural of anything we could be sure about.
        assert_eq!(key("oes"), "oes");
        assert_eq!(key("ies"), "ies");
    }

    /// A pantry holding both spellings is one jar, and says so with one of them —
    /// deterministically, so a pre-tick does not rename itself between reads.
    #[test]
    fn both_spellings_in_one_pantry_are_one_entry() {
        let p = pantry(&["onions", "onion"]);
        assert_eq!(p.holds("onion"), Some("onion"), "sorted-first entry wins");
        assert_eq!(p.holds("onions"), Some("onion"));
    }

    /// An empty pantry has nothing to say, and an empty entry is not an entry — a
    /// blank must never become a key that matches a blank ingredient name.
    #[test]
    fn an_empty_pantry_covers_nothing() {
        let p = pantry(&[]);
        assert!(p.is_empty());
        assert_eq!(p.holds("salt"), None);

        let blank = pantry(&["", "   "]);
        assert!(blank.is_empty(), "whitespace is not stock");
        assert_eq!(blank.holds(""), None);
    }

    /// The projection the checklist's indices count against: only lines with a reading
    /// and a name are on the list, so an unread line takes no index. If this ever
    /// disagreed with `getBuyList` in `frontend/src/lib/buy.ts`, pre-ticks would land
    /// on the wrong rows.
    #[test]
    fn drops_unread_lines_so_indices_match_the_shopping_list() {
        let ingredients = [
            ing("2 large onions", Some("onions")),
            ing("a splash of something", None),
            ing("salt", Some("salt")),
            ing("mystery", Some("   ")),
            ing("1 tbsp olive oil", Some("olive oil")),
        ];
        assert_eq!(
            shopping_names(&ingredients),
            vec!["onions".to_string(), "salt".into(), "olive oil".into()]
        );
    }

    /// End to end: the indices a seed writes are the checklist's, and each carries the
    /// entry that answered for it.
    #[test]
    fn preticks_name_the_line_and_the_jar() {
        let ingredients = [
            ing("2 large onions, finely chopped", Some("onions")),
            ing("a pinch of nothing", None),
            ing("1 tsp salt", Some("salt")),
            ing("400g chicken thighs", Some("chicken thighs")),
            ing("1 tbsp olive oil", Some("olive oil")),
        ];
        let names = shopping_names(&ingredients);
        // The unread line is gone, so "salt" is index 1 on the list, not index 2.
        assert_eq!(names, vec!["onions", "salt", "chicken thighs", "olive oil"]);
        assert_eq!(
            preticks(&names, &pantry(&["onion", "salt"])),
            vec![(0, "onion".to_string()), (1, "salt".to_string())]
        );
    }

    /// An empty pantry does no work and pre-ticks nothing — the production state today.
    #[test]
    fn an_empty_pantry_pre_ticks_nothing() {
        let names = vec!["salt".to_string(), "onion".into()];
        assert!(preticks(&names, &Pantry::default()).is_empty());
    }

    /// An unread recipe has no names on its list, so nothing is pre-ticked — never
    /// "nothing needed". The same ruling `equipment::Capability::Unread` makes.
    #[test]
    fn an_unread_recipe_pre_ticks_nothing() {
        let ingredients = [ing("salt", None), ing("onions", None)];
        let names = shopping_names(&ingredients);
        assert!(names.is_empty());
        assert!(preticks(&names, &pantry(&["salt", "onion"])).is_empty());
    }
}
