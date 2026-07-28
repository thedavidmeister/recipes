//! Normalized recipe types shared across all sources.
//!
//! Every source (TheMealDB today, future APIs) maps its own payload onto these
//! types so the frontend sees one consistent shape.

use serde::{Deserialize, Serialize};

use crate::equipment::RequiredEquipment;
use crate::measure::StructuredMeasure;
use crate::nutrition::{self, FoodEnergy, RecipeEnergy};
use crate::step::StructuredStep;

/// A single ingredient line. `measure` is the quantity/unit when the source
/// provides it separately (TheMealDB does); free-text sources fold it into
/// `name`.
///
/// These types derive both `Serialize` and `Deserialize` so they round-trip
/// across the wire (WASM ⇄ JS ⇄ backend) and in and out of Turso.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ingredient {
    pub name: String,
    pub measure: Option<String>,
    /// The LLM's structured reading of this line (#11), populated at ingestion by
    /// the backend's `enrich` step. The raw `name`/`measure` above stay the source
    /// of truth — this is an enrichment the UI can use for scaling and unit
    /// conversion, and falls back from if the model was wrong (parse-but-preserve).
    ///
    /// `None` until enrichment runs (or when no API key is configured — it degrades
    /// rather than blocking ingestion). `#[serde(default)]` so rows stored before
    /// this field existed still deserialize; `skip_serializing_if` so an un-enriched
    /// line stores exactly as it did before, no churn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<StructuredMeasure>,
}

/// A normalized recipe from any source.
///
/// Some arrive only partially populated — a browse/listing result (e.g.
/// TheMealDB `filter.php`) may carry just `id`, `source`, `title`, and `image`,
/// with the rest empty. Absent detail is represented as empty (`""` / `[]` /
/// `None`), not as a distinct "unknown".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    /// Source-specific identifier (opaque to the frontend).
    pub id: String,
    /// Which source this came from, e.g. `"themealdb"`.
    pub source: String,
    pub title: String,
    pub image: Option<String>,
    pub category: Option<String>,
    /// Cuisine / region of origin.
    pub area: Option<String>,
    pub tags: Vec<String>,
    pub ingredients: Vec<Ingredient>,
    pub instructions: String,
    /// The model's structured reading of `instructions` into a step DAG (#74/#75/
    /// #76), attached at derive from the `step_structures` capture — what `cook`
    /// renders. `instructions` above stays the source of truth (the reading's
    /// input), exactly as `Ingredient::measure` does beside `structured`.
    ///
    /// Empty until the step-reading worker runs (or when it hasn't read this
    /// recipe): `#[serde(default)]` so rows stored before this field existed still
    /// deserialize; `skip_serializing_if` so an un-read recipe stores as it did
    /// before, no churn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StructuredStep>,
    /// What the recipe needs you to own (#81).
    ///
    /// Empty until the equipment worker has read it — and, because a kitchen selects
    /// only from what recipes ask for, an un-read recipe contributes nothing to that
    /// vocabulary rather than contributing a guess.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equipment: Vec<RequiredEquipment>,
    /// The model's reading of each ingredient's food (#162), attached at derive from
    /// the `nutrition_structures` capture — one entry per ingredient, in ingredient
    /// order, carrying energy density and (where a unit table cannot say) the mass of
    /// one of the line's unit.
    ///
    /// Stored beside the recipe rather than only in its capture table for the same
    /// reason `steps` and `equipment` are: [`Self::energy`] is computed from this
    /// field in the same write that stores it, so the total can never describe a
    /// different reading from the one alongside it.
    ///
    /// Empty until the nutrition worker has read the recipe — degrade-not-die, and
    /// `skip_serializing_if` so an un-read recipe stores exactly as it did before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nutrition: Vec<FoodEnergy>,
    /// How many people this recipe feeds, as the model read it (#162).
    ///
    /// **Not a source field** — no source in the corpus carries a yield, so this is a
    /// reading like every other. It is captured because a bare calorie total is
    /// ambiguous in the way that matters most: 2,400 kcal is a reasonable tray of
    /// lasagne and an absurd plate of it. Per the #158 ruling, a recipe whose servings
    /// nobody has written down still feeds a definite number of people, so an absent
    /// count is a gap in our reading rather than a property of the dish.
    ///
    /// `None` only until the worker has read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servings: Option<u32>,
    /// Canonical URL of the recipe on its origin site, when known.
    pub source_url: Option<String>,
    pub video_url: Option<String>,
}

impl Recipe {
    /// This recipe's estimated total time start to finish, prep included (#79) — the
    /// critical path through its step DAG, or `None` when the steps carry no timing to
    /// sum (un-read, or nothing timed). A thin wrapper over [`step::total_seconds`]
    /// (crate::step) so the estimate lives with the graph it reads; `derive` computes it
    /// here and the `recipes` view stores it, keeping the arithmetic single-sourced in
    /// `recipe-core` — the browser reads Turso directly and cannot run this (no WASM).
    pub fn total_seconds(&self) -> Option<u32> {
        crate::step::total_seconds(&self.steps)
    }

    /// Whether every one of this recipe's steps carries a duration (#158/#84) — so
    /// whether [`Self::total_seconds`] is a complete estimate or only a lower bound.
    ///
    /// Stored beside the total for the same reason the total is stored at all: the
    /// browser reads Turso directly and cannot run `recipe-core` (no WASM,
    /// deliberately), and deciding this in JS would be a second definition of "is the
    /// number complete" free to drift from this one. Read off the same `steps` as the
    /// total, in the same place, so they cannot disagree.
    pub fn fully_timed(&self) -> bool {
        crate::step::fully_timed(&self.steps)
    }

    /// This recipe's energy (#162) — the sum over its ingredients of quantity ×
    /// gram weight × energy density — or `None` when nothing could be counted.
    ///
    /// The exact peer of [`Self::total_seconds`], and for the same reasons: the
    /// arithmetic lives in `recipe-core` beside the data it reads, `derive` computes
    /// it here and the `recipes` view stores the result, and the browser (which reads
    /// Turso directly and cannot run this crate — no WASM, deliberately) gets a
    /// number rather than a second implementation.
    ///
    /// It needs **both** readings: the ingredient reading (#11) for the quantities and
    /// the nutrition reading for the food. A recipe with one and not the other has no
    /// total, which is why the nutrition queue only offers recipes already enriched.
    pub fn energy(&self) -> Option<RecipeEnergy> {
        nutrition::recipe_energy(&self.ingredients, &self.nutrition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::{Amount, Quantity, StructuredMeasure};
    use crate::step::{StepKind, StructuredStep};

    /// An un-enriched line must serialize exactly as it did before the field
    /// existed — no `"structured"` key — so the 700-odd rows already in Turso
    /// don't churn and old rows still read back.
    #[test]
    fn structured_none_is_omitted_and_absent_deserializes_to_none() {
        let raw = Ingredient {
            name: "flour".into(),
            measure: Some("1 cup".into()),
            structured: None,
        };
        let json = serde_json::to_string(&raw).unwrap();
        assert!(
            !json.contains("structured"),
            "an un-enriched line must not write a structured key: {json}"
        );

        // A row stored before this field existed (no key at all) reads back None.
        let old: Ingredient =
            serde_json::from_str(r#"{"name":"flour","measure":"1 cup"}"#).unwrap();
        assert_eq!(old.structured, None);
    }

    /// A populated reading round-trips intact, with the raw text alongside it —
    /// parse-but-preserve, and the storage shape acceptance #1 asks for.
    #[test]
    fn structured_some_round_trips_beside_the_raw_text() {
        let ing = Ingredient {
            name: "chicken thighs".into(),
            measure: Some("500 g".into()),
            structured: Some(StructuredMeasure {
                item: "chicken thighs".into(),
                amount: Some(Amount::Quantified {
                    quantity: Quantity::Exact { value: 500.0 },
                    unit: Some("g".into()),
                    size: None,
                }),
                preparation: Some("diced".into()),
                note: None,
            }),
        };
        let back: Ingredient = serde_json::from_str(&serde_json::to_string(&ing).unwrap()).unwrap();
        assert_eq!(ing, back);
        // Raw stays the source of truth next to the enrichment.
        assert_eq!(back.measure.as_deref(), Some("500 g"));
    }

    fn recipe_with_steps(steps: Vec<StructuredStep>) -> Recipe {
        Recipe {
            id: "1".into(),
            source: "themealdb".into(),
            title: "T".into(),
            image: None,
            category: None,
            area: None,
            tags: vec![],
            ingredients: vec![],
            instructions: "go".into(),
            steps,
            equipment: vec![],
            nutrition: vec![],
            servings: None,
            source_url: None,
            video_url: None,
        }
    }

    /// `Recipe::total_seconds` reads the critical path off the recipe's own step DAG
    /// (#79) — the arithmetic `derive` stores on the `recipes` view. A read recipe with
    /// timed steps yields its longest path; an un-read one (no steps) yields `None`.
    #[test]
    fn recipe_total_seconds_reads_the_critical_path_of_its_steps() {
        let cook = |id: u32, seconds: Option<u32>, after: &[u32]| StructuredStep {
            id,
            text: format!("step {id}"),
            kind: StepKind::Cook,
            seconds,
            after: after.to_vec(),
        };

        // 0 (60) -> {1 (120), 2 (300)} -> 3 (30): critical path 60+300+30 = 390.
        let read = recipe_with_steps(vec![
            cook(0, Some(60), &[]),
            cook(1, Some(120), &[0]),
            cook(2, Some(300), &[0]),
            cook(3, Some(30), &[1, 2]),
        ]);
        assert_eq!(read.total_seconds(), Some(390));

        // An un-read recipe carries no steps, so there is no estimate.
        assert_eq!(recipe_with_steps(vec![]).total_seconds(), None);
    }

    /// `Recipe::energy` sums off the recipe's own two readings (#162) — the arithmetic
    /// `derive` stores on the `recipes` view. It needs **both**: an un-read recipe has
    /// no total, and neither does one whose nutrition reading has drifted out of
    /// alignment with its ingredient list.
    #[test]
    fn recipe_energy_needs_both_readings_and_sums_off_them() {
        let line = |grams: f64| Ingredient {
            name: "x".into(),
            measure: None,
            structured: Some(StructuredMeasure {
                item: "x".into(),
                amount: Some(Amount::Quantified {
                    quantity: Quantity::Exact { value: grams },
                    unit: Some("g".into()),
                    size: None,
                }),
                preparation: None,
                note: None,
            }),
        };
        let mut recipe = recipe_with_steps(vec![]);
        recipe.ingredients = vec![line(100.0), line(200.0)];

        // No nutrition reading yet: degrade-not-die, no number rather than a zero.
        assert_eq!(recipe.energy(), None);

        recipe.nutrition = vec![
            FoodEnergy {
                kcal_per_100g: 364.0,
                grams_per_unit: None,
            },
            FoodEnergy {
                kcal_per_100g: 209.0,
                grams_per_unit: None,
            },
        ];
        let energy = recipe.energy().expect("both readings present");
        assert!((energy.kcal - (364.0 + 418.0)).abs() < 1e-6);
        assert_eq!(energy.counted, 2);
        assert!(energy.complete());

        // A reading that no longer lines up with the ingredients is refused, not
        // summed over a prefix.
        recipe.ingredients.push(line(50.0));
        assert_eq!(recipe.energy(), None);
    }

    /// The two new fields are omitted from an un-read recipe's JSON, so the 790 rows
    /// already in Turso do not churn, and a row stored before they existed still reads
    /// back — the same contract `steps` and `equipment` hold.
    #[test]
    fn an_unread_nutrition_reading_is_omitted_and_absent_deserializes() {
        let recipe = recipe_with_steps(vec![]);
        let json = serde_json::to_string(&recipe).unwrap();
        assert!(
            !json.contains("nutrition") && !json.contains("servings"),
            "an un-read recipe must write neither key: {json}"
        );
        let back: Recipe = serde_json::from_str(&json).unwrap();
        assert!(back.nutrition.is_empty());
        assert_eq!(back.servings, None);
    }
}
