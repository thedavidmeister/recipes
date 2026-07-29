//! The nutrition reading (#162): roughly what a recipe costs you, in kcal.
//!
//! The fourth enrichment, and the one where the **split by strength** rule does the
//! most work. A recipe's energy is almost entirely arithmetic —
//! `Σ quantity × grams-per-unit × kcal-per-gram` — and CLAUDE.md is emphatic that the
//! model never adds up. So the split here is sharper than in the other three:
//!
//! - the **model** supplies, per ingredient line, two facts **about the food**:
//!   [`FoodEnergy::kcal_per_100g`] and, where a unit table cannot say it,
//!   [`FoodEnergy::grams_per_unit`]. Neither mentions this recipe's quantity;
//!   both would be the same reading in any other recipe using the same food.
//! - **this module** multiplies by the quantity the ingredient reading (#11) already
//!   captured, converts units off the table in [`crate::measure`], and sums.
//!
//! The model is therefore never asked for a number that grows with the recipe. Ask it
//! for "the calories in this recipe" and you get a plausible-looking total nobody can
//! check; ask it "how many kcal in 100 g of butter" and you get a fact, wrong by an
//! amount you could look up.
//!
//! ## Why the capture is per recipe, not per food name
//!
//! Energy density is a property of a food, so one reading of "flour" could in
//! principle serve all 790 recipes rather than being re-read for each of the ~8,000
//! ingredient lines. That is a real saving and it was seriously considered. It is not
//! what this does, for three reasons:
//!
//! 1. **Most of what the model supplies is not per-food at all.**
//!    [`FoodEnergy::grams_per_unit`] is the mass of one of *the line's own unit* — one
//!    clove, one cup, one can, one egg. The unit comes from the recipe line, so a
//!    per-food table would have to carry a map of every unit any recipe ever paired
//!    with that food: the same information, re-keyed, plus a join.
//! 2. **A food name is not a vocabulary.** The equipment reading (#81) could be keyed
//!    by name because a kitchen *picks from* those names, which forces them to
//!    normalise. Nothing forces ingredient names to normalise, and a per-name table
//!    would import that whole problem — "Plain Flour" and "plain flour" as two rows —
//!    against 948 distinct free-text names with no downstream user to keep them
//!    honest.
//! 3. **The cascade is per-`(source, id)` everywhere else.** The pull's left join, the
//!    push's count check, the `run_id` guard, and the targeted re-derive after a push
//!    are all keyed that way. A per-name table would have no answer to "which recipes
//!    must be re-derived now that 'flour' has been read" short of a scan.
//!
//! The cost is re-reading a food ~8× across the corpus. That is the model's time, and
//! the worker runs under a plan where marginal inference is ~$0 (#59) — so it buys
//! nothing to trade the cascade away for it. And it is not a door closed: a per-food
//! table can be *derived* from these readings later (group by item, compare the
//! readings, and any disagreement is itself a finding), which is strictly more than a
//! per-food capture would have given us.
//!
//! ## What this deliberately does not claim
//!
//! A calorie count is an estimate and must never render as a measurement (#84). Two
//! specific holes are built into the arithmetic rather than papered over:
//!
//! - A line with **no number** — "salt, to taste", "oil, for frying" — contributes
//!   nothing, because there is nothing to multiply. Salt genuinely costs nothing;
//!   frying oil genuinely does. We cannot tell them apart from the line, so the total
//!   is understated by an unknown amount on nearly every recipe, and the surface
//!   hedges once rather than pretending otherwise.
//! - A line with a number we **cannot turn into grams** is counted as a *gap*, not as
//!   zero — see [`RecipeEnergy::uncounted`]. That is the difference between "we know
//!   this contributes nothing" and "we failed to read this", and per the #158 ruling
//!   the second is a defect in our reading, never a property of the dish.

use serde::{Deserialize, Serialize};

use crate::measure::{grams_of, Amount, StructuredMeasure};
use crate::models::Ingredient;

/// The model's reading of one ingredient line's **food** — never of its quantity.
///
/// One entry per ingredient, in the recipe's ingredient order, exactly like the
/// [`StructuredMeasure`] reading it rides beside (#11). Alignment by position is what
/// makes the push's count check able to refuse a reading that has drifted from the
/// recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FoodEnergy {
    /// Energy density of the food as it goes into the pot, in kcal per 100 g.
    ///
    /// Per 100 g because that is how every nutrition label and every food database
    /// states it, so the model is recalling a figure rather than converting one. `0.0`
    /// is a legitimate reading — water and salt really are zero — and is not the same
    /// as an absent one, which is why this is not an `Option`: every food has an
    /// energy density, and not knowing it is a gap in the reading (#158).
    pub kcal_per_100g: f64,
    /// The mass in grams of **one** of whatever unit this line uses — one clove, one
    /// cup, one can, one egg — or `None` when the line's unit is already a mass unit.
    ///
    /// This is the single field that carries everything a unit table cannot know, and
    /// it is deliberately one field rather than a density plus a per-item weight: "how
    /// much does one cup of flour weigh" and "how much does one clove of garlic weigh"
    /// are the same question asked of different units, and folding them together means
    /// the model answers one question per line instead of choosing between two schemas.
    ///
    /// `None` where [`grams_of`] already knows the answer — the reading does not get to
    /// disagree with the unit table about how many grams are in an ounce, and
    /// [`line_kcal`] does not consult this when the unit is a mass unit even if it is
    /// supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grams_per_unit: Option<f64>,
}

/// The largest energy density anything edible has: pure fat, ~900 kcal/100 g.
///
/// A guard against the specific failure this schema invites — a model that answers per
/// serving, per pound, or per whole recipe. Those come back an order of magnitude high
/// and would otherwise be silently summed into a confident, absurd total.
pub const MAX_KCAL_PER_100G: f64 = 900.0;

/// The largest mass one unit of anything in a home recipe plausibly weighs: 20 kg.
///
/// Generous on purpose — a gallon of stock is ~3.8 kg and a whole leg of lamb ~3 kg,
/// so nothing real comes close. It exists to catch a misplaced decimal or a milligram
/// answer, not to police the reading.
pub const MAX_GRAMS_PER_UNIT: f64 = 20_000.0;

/// The most people one recipe is read as feeding.
///
/// A home recipe serving more than this has been read as a catering batch — the
/// failure mode where "500 g of pasta" becomes "serves 50". `1` is the floor because
/// a recipe feeding nobody is not a recipe.
pub const MAX_SERVINGS: u32 = 100;

/// Check one recipe's reading before it is stored.
///
/// Refuses rather than repairs, like every other push in this pipeline: a rejected
/// recipe re-enters the queue and is read again, which is strictly better than storing
/// a number that will be summed into a total nobody can audit.
pub fn validate(foods: &[FoodEnergy], servings: u32) -> Result<(), String> {
    if !(1..=MAX_SERVINGS).contains(&servings) {
        return Err(format!(
            "servings is {servings} — must be between 1 and {MAX_SERVINGS}; a home recipe \
             feeding more than that has been read as a catering batch"
        ));
    }
    for (i, food) in foods.iter().enumerate() {
        if !food.kcal_per_100g.is_finite() || food.kcal_per_100g < 0.0 {
            return Err(format!(
                "ingredient {i} has kcal_per_100g {} — must be a finite, non-negative number",
                food.kcal_per_100g
            ));
        }
        if food.kcal_per_100g > MAX_KCAL_PER_100G {
            return Err(format!(
                "ingredient {i} has kcal_per_100g {} — nothing edible exceeds {MAX_KCAL_PER_100G} \
                 (pure fat); this reads like a per-serving or per-pound figure",
                food.kcal_per_100g
            ));
        }
        if let Some(grams) = food.grams_per_unit {
            if !grams.is_finite() || grams <= 0.0 {
                return Err(format!(
                    "ingredient {i} has grams_per_unit {grams} — one of something weighs more \
                     than nothing"
                ));
            }
            if grams > MAX_GRAMS_PER_UNIT {
                return Err(format!(
                    "ingredient {i} has grams_per_unit {grams} — above {MAX_GRAMS_PER_UNIT} g for \
                     one unit, which reads like a misplaced decimal"
                ));
            }
        }
    }
    Ok(())
}

/// The mass in grams of one of this amount's unit, or `None` when nothing here can
/// say.
///
/// **Deterministic code first, the reading only where code cannot answer.** The ladder
/// is the whole split-by-strength rule made executable:
///
/// 1. The **unit is a mass unit** — [`grams_of`] is exact, and the reading is not
///    consulted at all. A model that thought an ounce was 30 g cannot make it so.
/// 2. The line carries a **size annotation in a mass unit** — `"1 (14 oz) can"`,
///    `"3 tbsp (45 g) butter"`. The source stated the mass; multiplying it out is
///    arithmetic, so we do it rather than asking.
/// 3. The **reading's [`FoodEnergy::grams_per_unit`]** — a cup of flour, a clove of
///    garlic, an egg. Nothing but knowing the food can answer this.
/// 4. Otherwise `None`: a number we cannot weigh. Not zero — a gap.
fn grams_per_unit(amount: &Amount, food: &FoodEnergy) -> Option<f64> {
    let Amount::Quantified { unit, size, .. } = amount else {
        return None;
    };
    if let Some(grams) = unit.as_deref().and_then(grams_of) {
        return Some(grams);
    }
    if let Some(size) = size {
        if let Some(grams) = size.unit.as_deref().and_then(grams_of) {
            // "1 (14 oz) can" — one can is 14 × 28.35 g. The size describes the
            // package, so this is the mass of one unit, exactly what we want.
            return Some(size.quantity.midpoint() * grams);
        }
    }
    food.grams_per_unit
}

/// Whether deterministic code can already weigh this line, so the reading does not
/// need to supply a [`FoodEnergy::grams_per_unit`] for it.
///
/// The pull hands this to the worker per line, so the model is asked for a gram weight
/// only where nothing else can answer. It is defined by running the same ladder
/// [`line_kcal`] runs, against a reading that supplies nothing — so the hint and the
/// sum can never disagree about which lines code covers.
pub fn weighable_without_reading(measure: &StructuredMeasure) -> bool {
    let Some(amount) = measure.amount.as_ref() else {
        return false;
    };
    grams_per_unit(
        amount,
        &FoodEnergy {
            kcal_per_100g: 0.0,
            grams_per_unit: None,
        },
    )
    .is_some()
}

/// The energy one ingredient line contributes, in kcal, or `None` when it cannot be
/// counted.
///
/// `None` has two quite different causes and the caller has to tell them apart:
///
/// - the amount is **absent or qualitative** ("to taste", "a pinch") — there is no
///   number, so there is nothing to multiply and never was;
/// - the amount **has a number we could not weigh** — a unit no table knows and no
///   reading covered. That is a gap.
///
/// [`recipe_energy`] does the telling apart; this returns the number or nothing.
pub fn line_kcal(measure: &StructuredMeasure, food: &FoodEnergy) -> Option<f64> {
    let amount = measure.amount.as_ref()?;
    let Amount::Quantified { quantity, .. } = amount else {
        return None;
    };
    let grams = quantity.midpoint() * grams_per_unit(amount, food)?;
    Some(grams * food.kcal_per_100g / 100.0)
}

/// A recipe's energy, with the honesty about it that #84 requires.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecipeEnergy {
    /// Total kcal for the **whole recipe**, summed over the lines that could be
    /// counted. Never per serving: dividing is the surface's job, and it needs the
    /// servings reading beside it to do so.
    pub kcal: f64,
    /// How many ingredient lines contributed a number.
    pub counted: usize,
    /// How many lines stated a number we could **not** turn into grams.
    ///
    /// The measure of how much of this total is missing, and the reason
    /// [`RecipeEnergy::complete`] exists. Lines with no number at all are *not* counted
    /// here: "to taste" is not a failed reading, it is a line that never had a quantity
    /// — flagging every recipe with a pinch of salt would make the flag useless and
    /// say nothing true.
    pub uncounted: usize,
}

impl RecipeEnergy {
    /// Whether every line that stated a number was counted — so whether this total is
    /// an estimate or a floor.
    ///
    /// The peer of `fully_timed` (#158/#84) and it exists for the same reason: the
    /// browser reads Turso directly and cannot run this crate (no WASM, deliberately),
    /// so "is this number complete" is decided once, here, beside the number itself.
    pub fn complete(&self) -> bool {
        self.uncounted == 0
    }
}

/// Sum a recipe's energy from its ingredient readings and its nutrition reading.
///
/// Both readings are required and must be aligned: `foods[i]` describes
/// `ingredients[i]`. A length mismatch returns `None` rather than summing a prefix —
/// misaligned readings produce a number that looks fine and is about a different
/// recipe, which is worse than no number. The push refuses to store one, so this only
/// fires if a recipe's ingredient list changed under a stored reading.
///
/// `None` also when **nothing at all could be counted**: an unread recipe, or one
/// whose every line is "to taste". A `0.0` there would render as a recipe with no
/// calories, which is never true of food — an absent number is the honest answer
/// (#158) and the same call `total_seconds` makes.
pub fn recipe_energy(ingredients: &[Ingredient], foods: &[FoodEnergy]) -> Option<RecipeEnergy> {
    if foods.is_empty() || foods.len() != ingredients.len() {
        return None;
    }
    let mut energy = RecipeEnergy {
        kcal: 0.0,
        counted: 0,
        uncounted: 0,
    };
    for (ingredient, food) in ingredients.iter().zip(foods) {
        // No ingredient reading means no quantity, so there is nothing to multiply —
        // the same "no number" case as "to taste", and not a nutrition gap.
        let Some(measure) = ingredient.structured.as_ref() else {
            continue;
        };
        match line_kcal(measure, food) {
            Some(kcal) => {
                energy.kcal += kcal;
                energy.counted += 1;
            }
            // A stated number we could not weigh is the gap; a line with no number
            // never had one to lose.
            None if states_a_number(measure) => energy.uncounted += 1,
            None => {}
        }
    }
    (energy.counted > 0).then_some(energy)
}

/// Whether this line put a number on its amount — the test that separates "we failed
/// to read this" from "there was nothing to read".
fn states_a_number(measure: &StructuredMeasure) -> bool {
    matches!(measure.amount, Some(Amount::Quantified { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::{Quantity, Size};

    fn food(kcal_per_100g: f64, grams_per_unit: Option<f64>) -> FoodEnergy {
        FoodEnergy {
            kcal_per_100g,
            grams_per_unit,
        }
    }

    fn exact(value: f64) -> Quantity {
        Quantity::Exact { value }
    }

    fn measure(amount: Option<Amount>) -> StructuredMeasure {
        StructuredMeasure {
            item: "x".into(),
            amount,
            preparation: None,
            note: None,
        }
    }

    fn quantified(quantity: Quantity, unit: Option<&str>, size: Option<Size>) -> Amount {
        Amount::Quantified {
            quantity,
            unit: unit.map(str::to_owned),
            size,
        }
    }

    fn near(got: Option<f64>, expected: f64) {
        let got = got.expect("expected a number");
        assert!(
            (got - expected).abs() < 1e-6,
            "got {got}, expected {expected}"
        );
    }

    fn ingredient(structured: Option<StructuredMeasure>) -> Ingredient {
        Ingredient {
            name: "x".into(),
            measure: None,
            structured,
        }
    }

    /// A mass unit needs no reading at all: `grams_of` is exact, and the arithmetic is
    /// quantity × grams × density. 500 g of chicken thigh at 209 kcal/100 g.
    #[test]
    fn a_mass_unit_is_weighed_by_the_unit_table_alone() {
        let m = measure(Some(quantified(exact(500.0), Some("g"), None)));
        near(line_kcal(&m, &food(209.0, None)), 1045.0);

        // …and an ounce is 28.3495 g, from the same table the converter uses.
        let oz = measure(Some(quantified(exact(4.0), Some("oz"), None)));
        near(line_kcal(&oz, &food(100.0, None)), 113.398_08);
    }

    /// **The reading does not get to disagree with the unit table.** A model claiming
    /// an ounce is 30 g changes nothing — rung 1 of the ladder never consults it. This
    /// is the split-by-strength rule as a test.
    #[test]
    fn a_reading_cannot_override_the_unit_table_for_a_mass_unit() {
        let m = measure(Some(quantified(exact(1.0), Some("oz"), None)));
        let with_a_wrong_reading = food(100.0, Some(30.0));
        near(line_kcal(&m, &with_a_wrong_reading), 28.349_52);
    }

    /// A volume, a count and a bare count all need the food known — that is exactly
    /// what `grams_per_unit` carries, and one field covers all three.
    #[test]
    fn volume_counts_and_bare_counts_come_from_the_reading() {
        // 1 cup of flour ≈ 125 g at 364 kcal/100 g.
        let cup = measure(Some(quantified(exact(1.0), Some("cup"), None)));
        near(line_kcal(&cup, &food(364.0, Some(125.0))), 455.0);

        // 2 cloves of garlic, ~3 g each at 149 kcal/100 g.
        let cloves = measure(Some(quantified(exact(2.0), Some("clove"), None)));
        near(line_kcal(&cloves, &food(149.0, Some(3.0))), 8.94);

        // "2 eggs" — no unit at all, 50 g each at 143 kcal/100 g.
        let eggs = measure(Some(quantified(exact(2.0), None, None)));
        near(line_kcal(&eggs, &food(143.0, Some(50.0))), 143.0);
    }

    /// A size annotation in a mass unit is the source stating the mass, so code
    /// multiplies it out rather than asking — and it beats the reading, because the
    /// line is more specific than a general fact about the food.
    #[test]
    fn a_mass_size_annotation_beats_the_reading() {
        // "1 (14 oz) can" of chopped tomatoes at 32 kcal/100 g: 14 oz = 396.893 g.
        let can = measure(Some(quantified(
            exact(1.0),
            Some("can"),
            Some(Size {
                quantity: exact(14.0),
                unit: Some("oz".into()),
            }),
        )));
        // The reading's 500 g/can is ignored in favour of the stated 14 oz.
        near(line_kcal(&can, &food(32.0, Some(500.0))), 127.005_849_6);

        // A size in a *volume* unit cannot be weighed either, so it falls through to
        // the reading — 400 ml of coconut milk the model weighs at 405 g/can.
        let ml_can = measure(Some(quantified(
            exact(1.0),
            Some("can"),
            Some(Size {
                quantity: exact(400.0),
                unit: Some("ml".into()),
            }),
        )));
        near(line_kcal(&ml_can, &food(230.0, Some(405.0))), 931.5);
    }

    /// A range is one number for this sum, and it is the midpoint — "2-3 cloves" costs
    /// what 2.5 cloves cost.
    #[test]
    fn a_range_is_summed_at_its_midpoint() {
        let m = measure(Some(quantified(
            Quantity::Range {
                low: 2.0,
                high: 3.0,
            },
            Some("clove"),
            None,
        )));
        near(line_kcal(&m, &food(100.0, Some(4.0))), 10.0);
    }

    /// **Scaling composes for free** — the whole reason the model is asked for a
    /// per-unit figure and never a total. Doubling the recipe doubles the quantity;
    /// the food's density and gram weight are untouched, so the calories double with
    /// no nutrition-specific scaling code at all.
    #[test]
    fn scaling_the_measure_scales_the_calories() {
        let f = food(364.0, Some(125.0));
        let one = measure(Some(quantified(exact(1.0), Some("cup"), None)));
        let base = line_kcal(&one, &f).unwrap();
        near(line_kcal(&one.scaled(2.0), &f), base * 2.0);
        near(line_kcal(&one.scaled(0.5), &f), base / 2.0);
    }

    /// A line with no number contributes nothing and never could — there is nothing to
    /// multiply, whatever the food is.
    #[test]
    fn a_line_with_no_number_yields_nothing() {
        assert_eq!(line_kcal(&measure(None), &food(900.0, Some(10.0))), None);
        let to_taste = measure(Some(Amount::Qualitative {
            text: "to taste".into(),
        }));
        assert_eq!(line_kcal(&to_taste, &food(900.0, Some(10.0))), None);
    }

    /// A number in a unit nothing can weigh is `None`, **not zero**. Guessing a gram
    /// weight here is exactly the wrong-number-looks-fine failure the whole ladder
    /// exists to avoid.
    #[test]
    fn a_number_in_an_unweighable_unit_yields_nothing_rather_than_zero() {
        let m = measure(Some(quantified(exact(1.0), Some("cup"), None)));
        assert_eq!(line_kcal(&m, &food(364.0, None)), None);
    }

    /// Zero is a real reading, not a missing one: water and salt cost nothing, and
    /// that line is *counted*, because we know what it contributes.
    #[test]
    fn zero_kcal_is_a_reading_and_counts() {
        let water = measure(Some(quantified(exact(500.0), Some("ml"), None)));
        near(line_kcal(&water, &food(0.0, Some(1.0))), 0.0);

        let energy = recipe_energy(&[ingredient(Some(water))], &[food(0.0, Some(1.0))]).unwrap();
        assert_eq!(energy.counted, 1, "a known zero is a counted line");
        assert_eq!(energy.uncounted, 0);
        assert!(energy.complete());
    }

    /// The whole-recipe sum, and the honesty that rides with it: three lines, one of
    /// them un-weighable, so the total is a floor and says so.
    #[test]
    fn a_recipe_sums_its_lines_and_reports_the_gap() {
        let ingredients = vec![
            // 200 g chicken @ 209
            ingredient(Some(measure(Some(quantified(
                exact(200.0),
                Some("g"),
                None,
            ))))),
            // 1 cup flour @ 364, 125 g/cup
            ingredient(Some(measure(Some(quantified(
                exact(1.0),
                Some("cup"),
                None,
            ))))),
            // 1 splash of something — a number, in a unit nothing can weigh, with no
            // reading for it. This is the gap.
            ingredient(Some(measure(Some(quantified(
                exact(1.0),
                Some("splash"),
                None,
            ))))),
        ];
        let foods = vec![
            food(209.0, None),
            food(364.0, Some(125.0)),
            food(884.0, None),
        ];

        let energy = recipe_energy(&ingredients, &foods).unwrap();
        near(Some(energy.kcal), 418.0 + 455.0);
        assert_eq!(energy.counted, 2);
        assert_eq!(energy.uncounted, 1);
        assert!(
            !energy.complete(),
            "a line we could not weigh makes the total a floor"
        );
    }

    /// "To taste" is not a gap. A recipe whose only unread line has no number at all
    /// is *complete* — flagging it would make the flag meaningless, since almost every
    /// recipe has a pinch of something.
    #[test]
    fn a_qualitative_line_is_not_counted_as_a_gap() {
        let ingredients = vec![
            ingredient(Some(measure(Some(quantified(
                exact(100.0),
                Some("g"),
                None,
            ))))),
            ingredient(Some(measure(Some(Amount::Qualitative {
                text: "to taste".into(),
            })))),
        ];
        let energy = recipe_energy(&ingredients, &[food(100.0, None), food(0.0, None)]).unwrap();
        assert_eq!(energy.counted, 1);
        assert_eq!(energy.uncounted, 0);
        assert!(energy.complete());
    }

    /// A line with no *ingredient* reading has no quantity either, so it is the same
    /// "no number" case — nutrition cannot be read ahead of #11.
    #[test]
    fn a_line_with_no_ingredient_reading_is_not_a_nutrition_gap() {
        let ingredients = vec![
            ingredient(Some(measure(Some(quantified(
                exact(100.0),
                Some("g"),
                None,
            ))))),
            ingredient(None),
        ];
        let energy = recipe_energy(&ingredients, &[food(100.0, None), food(500.0, None)]).unwrap();
        assert_eq!(energy.counted, 1);
        assert_eq!(energy.uncounted, 0);
    }

    /// Misaligned readings are refused outright rather than summed over a prefix: a
    /// plausible total about a different recipe is worse than no total.
    #[test]
    fn a_misaligned_reading_yields_nothing() {
        let ingredients = vec![
            ingredient(Some(measure(Some(quantified(
                exact(100.0),
                Some("g"),
                None,
            ))))),
            ingredient(Some(measure(Some(quantified(
                exact(100.0),
                Some("g"),
                None,
            ))))),
        ];
        assert_eq!(recipe_energy(&ingredients, &[food(100.0, None)]), None);
        assert_eq!(
            recipe_energy(
                &ingredients,
                &[food(1.0, None), food(1.0, None), food(1.0, None)]
            ),
            None
        );
    }

    /// An unread recipe, and one whose every line is qualitative, both yield `None` —
    /// never `0.0`, which would render as a dish with no calories.
    #[test]
    fn nothing_countable_yields_no_number_rather_than_zero() {
        let ingredients = vec![ingredient(Some(measure(Some(Amount::Qualitative {
            text: "to taste".into(),
        }))))];
        assert_eq!(recipe_energy(&ingredients, &[food(0.0, None)]), None);
        // Unread: no nutrition reading at all.
        assert_eq!(recipe_energy(&ingredients, &[]), None);
    }

    /// The bounds catch the failure this schema invites: a per-serving or per-pound
    /// figure smuggled in as an energy density, and a misplaced decimal on a weight.
    #[test]
    fn validate_refuses_impossible_readings() {
        assert!(validate(&[food(364.0, Some(125.0))], 4).is_ok());
        assert!(
            validate(&[food(0.0, None)], 1).is_ok(),
            "zero is legitimate"
        );

        // 2,400 kcal/100 g is a whole recipe's total wearing a density's clothes.
        assert!(validate(&[food(2400.0, None)], 4).is_err());
        assert!(validate(&[food(-1.0, None)], 4).is_err());
        assert!(validate(&[food(f64::NAN, None)], 4).is_err());
        assert!(validate(&[food(f64::INFINITY, None)], 4).is_err());

        // One of something weighs more than nothing, and less than 20 kg.
        assert!(validate(&[food(100.0, Some(0.0))], 4).is_err());
        assert!(validate(&[food(100.0, Some(-5.0))], 4).is_err());
        assert!(validate(&[food(100.0, Some(1e9))], 4).is_err());
        assert!(validate(&[food(100.0, Some(f64::NAN))], 4).is_err());

        // Exactly at the bounds is fine — they are limits, not exclusions.
        assert!(validate(&[food(MAX_KCAL_PER_100G, Some(MAX_GRAMS_PER_UNIT))], 1).is_ok());
    }

    /// Servings must be a number a home recipe could mean.
    #[test]
    fn validate_refuses_impossible_servings() {
        assert!(validate(&[], 1).is_ok());
        assert!(validate(&[], MAX_SERVINGS).is_ok());
        assert!(validate(&[], 0).is_err(), "a recipe feeds somebody");
        assert!(validate(&[], MAX_SERVINGS + 1).is_err());
    }

    /// The hint the pull sends the worker is the same ladder the sum runs, so the model
    /// is asked for a gram weight exactly where code has no answer — and never asked to
    /// second-guess one it does.
    #[test]
    fn weighable_without_reading_matches_what_code_can_already_do() {
        // Mass units, and a mass size annotation: code has these.
        assert!(weighable_without_reading(&measure(Some(quantified(
            exact(500.0),
            Some("g"),
            None
        )))));
        assert!(weighable_without_reading(&measure(Some(quantified(
            exact(1.0),
            Some("can"),
            Some(Size {
                quantity: exact(14.0),
                unit: Some("oz".into())
            })
        )))));

        // A volume, a count, a bare count and an unknown unit all need the food known.
        for unit in [Some("cup"), Some("clove"), None, Some("splash")] {
            assert!(
                !weighable_without_reading(&measure(Some(quantified(exact(1.0), unit, None)))),
                "{unit:?} needs a gram weight from the reading"
            );
        }

        // Nothing to weigh at all.
        assert!(!weighable_without_reading(&measure(None)));
        assert!(!weighable_without_reading(&measure(Some(
            Amount::Qualitative {
                text: "to taste".into()
            }
        ))));
    }

    /// The reading round-trips through JSON — it is stored as JSON and read back by
    /// `derive`, so the wire shape is part of the contract. An absent
    /// `grams_per_unit` stays absent rather than churning every mass-unit line.
    #[test]
    fn food_energy_round_trips_through_json() {
        let with = food(364.0, Some(125.0));
        let json = serde_json::to_string(&with).unwrap();
        assert_eq!(serde_json::from_str::<FoodEnergy>(&json).unwrap(), with);

        let without = food(209.0, None);
        let json = serde_json::to_string(&without).unwrap();
        assert!(
            !json.contains("grams_per_unit"),
            "an absent gram weight must not write a key: {json}"
        );
        assert_eq!(serde_json::from_str::<FoodEnergy>(&json).unwrap(), without);
    }
}
