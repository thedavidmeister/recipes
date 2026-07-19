//! Structured ingredient measures — the enrichment for #11.
//!
//! A raw measure line is free text: `"1/2 cup"`, `"1 (14 oz) can"`, `"2-3 cloves,
//! minced"`, `"a pinch"`, `"to taste"`. You cannot scale it, convert its units, or
//! build a shopping list from it. An LLM at ingestion (the backend's `enrich`
//! step, #11) reads each line into this structured form; the **raw** string stays
//! the source of truth on [`crate::Ingredient`] and the UI falls back to it if the
//! model got a line wrong — parse-but-preserve.
//!
//! **Split by strength.** The LLM does the *extraction* (messy text → this shape).
//! The arithmetic — [`StructuredMeasure::scaled`] and
//! [`StructuredMeasure::converted`] — is **deterministic code here**, because a
//! model fumbles arithmetic and a unit table does not. This module has no LLM in
//! it; it is pure data + math and fully testable on its own.

use serde::{Deserialize, Serialize};

/// One ingredient line read into structure — an enrichment over the raw
/// `name`+`measure`, never a replacement.
///
/// Serialized enums are internally tagged (`{"kind": "exact", …}`) so the shape is
/// a plain JSON schema the extractor can constrain the model to, and so it
/// round-trips through Turso unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredMeasure {
    /// The ingredient itself, as the model read it (e.g. `"chicken thighs"`).
    /// Never invented — constrained to what the source line named. It can refine a
    /// messy name: a raw line might be the whole `"1 cup flour"`, and `item`
    /// pulls `"flour"` out of it.
    pub item: String,
    /// How much, or `None` for a bare ingredient with no stated amount.
    pub amount: Option<Amount>,
    /// Preparation folded into the line: `"minced"`, `"finely chopped"`.
    pub preparation: Option<String>,
    /// Anything else the line carried: `"to serve"`, `"optional"`, `"plus extra"`.
    pub note: Option<String>,
}

/// An amount: either a number (with an optional unit and size) or a qualitative
/// phrase that has no number at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Amount {
    /// A number, optionally with a unit and a size annotation.
    Quantified {
        quantity: Quantity,
        /// `"cup"`, `"g"`, `"clove"`, `"can"`. `None` for a bare count (`"2 eggs"`).
        unit: Option<String>,
        /// A size annotation — `"1 (14 oz) can"` is quantity 1, unit `"can"`, size
        /// the 14-oz reading. A size is always a plain number+unit (never itself
        /// qualitative or nested), which keeps this type non-recursive — a
        /// requirement for constraining the model with a JSON schema.
        size: Option<Size>,
    },
    /// A phrase with no number: `"to taste"`, `"a pinch"`, `"a splash"`.
    Qualitative { text: String },
}

/// The size inside an annotation like `"1 (14 oz) can"` — a number and a unit,
/// nothing more.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub quantity: Quantity,
    pub unit: Option<String>,
}

/// A numeric quantity: an exact value, or an inclusive range (`"2-3 cloves"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Quantity {
    Exact { value: f64 },
    Range { low: f64, high: f64 },
}

impl Quantity {
    /// Multiply through by `factor` — the primitive scaling and conversion both
    /// build on. A range scales at both ends.
    pub fn scaled(&self, factor: f64) -> Quantity {
        match *self {
            Quantity::Exact { value } => Quantity::Exact {
                value: value * factor,
            },
            Quantity::Range { low, high } => Quantity::Range {
                low: low * factor,
                high: high * factor,
            },
        }
    }
}

impl Amount {
    /// The amount at `factor`× the recipe (doubling, halving). A qualitative amount
    /// (`"to taste"`) is unchanged — twice "to taste" is still "to taste". A size
    /// annotation is per-unit and does **not** scale: two `"(14 oz) can"`s are two
    /// cans of the same 14 oz, so only the *count* scales.
    pub fn scaled(&self, factor: f64) -> Amount {
        match self {
            Amount::Quantified {
                quantity,
                unit,
                size,
            } => Amount::Quantified {
                quantity: quantity.scaled(factor),
                unit: unit.clone(),
                size: size.clone(),
            },
            Amount::Qualitative { text } => Amount::Qualitative { text: text.clone() },
        }
    }

    /// The amount expressed in `to_unit`, or `None` when the conversion is not
    /// defined: a qualitative amount, one with no unit, an unknown unit, or a
    /// change of dimension (a count of cloves cannot become grams — that needs a
    /// density this crate does not carry). The size annotation is carried through
    /// unchanged; it describes the package, not the amount being converted.
    pub fn converted(&self, to_unit: &str) -> Option<Amount> {
        let Amount::Quantified {
            quantity,
            unit,
            size,
        } = self
        else {
            return None;
        };
        let (from_dim, from_base) = parse_unit(unit.as_deref()?)?;
        let (to_dim, to_base) = parse_unit(to_unit)?;
        if from_dim != to_dim {
            return None;
        }
        Some(Amount::Quantified {
            quantity: quantity.scaled(from_base / to_base),
            unit: Some(to_unit.trim().to_lowercase()),
            size: size.clone(),
        })
    }
}

impl StructuredMeasure {
    /// The measure at `factor`× — scales the amount, leaves item/prep/note alone.
    pub fn scaled(&self, factor: f64) -> StructuredMeasure {
        StructuredMeasure {
            amount: self.amount.as_ref().map(|a| a.scaled(factor)),
            ..self.clone()
        }
    }

    /// The measure with its amount converted to `to_unit`, or `None` when the
    /// amount cannot be converted (see [`Amount::converted`]).
    pub fn converted(&self, to_unit: &str) -> Option<StructuredMeasure> {
        Some(StructuredMeasure {
            amount: Some(self.amount.as_ref()?.converted(to_unit)?),
            ..self.clone()
        })
    }
}

/// The two dimensions this crate converts within. A conversion crossing them
/// (volume ↔ mass) needs a per-ingredient density, which is out of scope — those
/// return `None` rather than a wrong number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dimension {
    Volume,
    Mass,
}

/// Parse a unit into `(dimension, factor-to-base)`, where the base is millilitres
/// for volume and grams for mass. `None` for anything not in the table — counts
/// (`"clove"`, `"can"`), qualitative words, or a typo. Case- and plural-tolerant,
/// and a trailing `.` (`"oz."`) is ignored.
///
/// `"oz"` is mass (ounce) and `"fl oz"` is volume — a distinction recipes rely on,
/// so they are separate entries rather than one fuzzy match.
fn parse_unit(s: &str) -> Option<(Dimension, f64)> {
    let u = s.trim().to_lowercase();
    let u = u.trim_end_matches('.');
    let entry = match u {
        // Volume — base millilitre.
        "ml" | "milliliter" | "milliliters" | "millilitre" | "millilitres" => {
            (Dimension::Volume, 1.0)
        }
        "l" | "liter" | "liters" | "litre" | "litres" => (Dimension::Volume, 1000.0),
        "tsp" | "teaspoon" | "teaspoons" => (Dimension::Volume, 4.928_92),
        "tbsp" | "tbs" | "tablespoon" | "tablespoons" => (Dimension::Volume, 14.786_76),
        "fl oz" | "fl. oz" | "fluid ounce" | "fluid ounces" => (Dimension::Volume, 29.573_53),
        "cup" | "cups" => (Dimension::Volume, 236.588_2),
        "pt" | "pint" | "pints" => (Dimension::Volume, 473.176_5),
        "qt" | "quart" | "quarts" => (Dimension::Volume, 946.352_9),
        "gal" | "gallon" | "gallons" => (Dimension::Volume, 3785.412),
        // Mass — base gram.
        "mg" | "milligram" | "milligrams" => (Dimension::Mass, 0.001),
        "g" | "gram" | "grams" => (Dimension::Mass, 1.0),
        "kg" | "kilogram" | "kilograms" => (Dimension::Mass, 1000.0),
        "oz" | "ounce" | "ounces" => (Dimension::Mass, 28.349_52),
        "lb" | "lbs" | "pound" | "pounds" => (Dimension::Mass, 453.592_4),
        _ => return None,
    };
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(value: f64) -> Quantity {
        Quantity::Exact { value }
    }

    fn amount(quantity: Quantity, unit: &str) -> Amount {
        Amount::Quantified {
            quantity,
            unit: Some(unit.into()),
            size: None,
        }
    }

    /// A converted quantity should equal the expected value within float slop.
    fn assert_quantity_near(q: &Quantity, expected: f64) {
        match q {
            Quantity::Exact { value } => assert!(
                (value - expected).abs() < 1e-3,
                "got {value}, expected {expected}"
            ),
            other => panic!("expected an exact quantity, got {other:?}"),
        }
    }

    #[test]
    fn scaling_multiplies_exact_and_range_but_not_qualitative() {
        assert_eq!(exact(2.0).scaled(3.0), exact(6.0));
        assert_eq!(
            Quantity::Range {
                low: 2.0,
                high: 3.0
            }
            .scaled(2.0),
            Quantity::Range {
                low: 4.0,
                high: 6.0
            }
        );
        // "to taste" doubled is still "to taste".
        let q = Amount::Qualitative {
            text: "to taste".into(),
        };
        assert_eq!(q.scaled(2.0), q);
    }

    #[test]
    fn scaling_a_size_annotation_scales_the_count_not_the_package() {
        // "1 (14 oz) can" doubled → 2 cans, each still 14 oz.
        let a = Amount::Quantified {
            quantity: exact(1.0),
            unit: Some("can".into()),
            size: Some(Size {
                quantity: exact(14.0),
                unit: Some("oz".into()),
            }),
        };
        let scaled = a.scaled(2.0);
        match scaled {
            Amount::Quantified {
                quantity,
                size: Some(size),
                ..
            } => {
                assert_eq!(quantity, exact(2.0), "the count doubles");
                assert_eq!(size.quantity, exact(14.0), "the can size does not");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn converts_within_a_dimension_and_round_trips() {
        // 1 cup → 236.588 ml, and back.
        let ml = amount(exact(1.0), "cup").converted("ml").unwrap();
        match &ml {
            Amount::Quantified { quantity, unit, .. } => {
                assert_quantity_near(quantity, 236.588);
                assert_eq!(unit.as_deref(), Some("ml"));
            }
            other => panic!("unexpected {other:?}"),
        }
        let back = ml.converted("cup").unwrap();
        if let Amount::Quantified { quantity, .. } = back {
            assert_quantity_near(&quantity, 1.0);
        }
        // Mass too: 100 g → ~3.527 oz.
        let oz = amount(exact(100.0), "g").converted("oz").unwrap();
        if let Amount::Quantified { quantity, .. } = oz {
            assert_quantity_near(&quantity, 3.5274);
        }
    }

    #[test]
    fn conversion_is_case_plural_and_abbreviation_tolerant() {
        // "Cups" → "Grams"? No — different dimension. But "Cups" parses.
        assert!(amount(exact(2.0), "Cups").converted("ML").is_some());
        assert!(amount(exact(1.0), "Tablespoon").converted("tsp").is_some());
        assert!(amount(exact(1.0), "oz.").converted("g").is_some());
    }

    #[test]
    fn conversion_refuses_across_dimensions_and_unknown_units() {
        // Volume → mass needs a density this crate doesn't have.
        assert!(amount(exact(1.0), "cup").converted("g").is_none());
        // "fl oz" is volume, "oz" is mass — not interchangeable.
        assert!(amount(exact(1.0), "fl oz").converted("oz").is_none());
        // A count is not a measure.
        assert!(amount(exact(2.0), "clove").converted("g").is_none());
        assert!(amount(exact(1.0), "cup").converted("clove").is_none());
    }

    #[test]
    fn conversion_refuses_when_there_is_nothing_to_convert() {
        // No unit.
        let bare = Amount::Quantified {
            quantity: exact(2.0),
            unit: None,
            size: None,
        };
        assert!(bare.converted("g").is_none());
        // Qualitative.
        assert!(Amount::Qualitative {
            text: "a pinch".into()
        }
        .converted("g")
        .is_none());
    }

    #[test]
    fn structured_measure_round_trips_through_json_with_every_variant() {
        // Cover the tagged enums (Exact, Range, Qualitative) and a size annotation.
        let m = StructuredMeasure {
            item: "chopped tomatoes".into(),
            amount: Some(Amount::Quantified {
                quantity: Quantity::Range {
                    low: 2.0,
                    high: 3.0,
                },
                unit: Some("can".into()),
                size: Some(Size {
                    quantity: exact(14.0),
                    unit: Some("oz".into()),
                }),
            }),
            preparation: Some("drained".into()),
            note: Some("plum, if you can find them".into()),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: StructuredMeasure = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);

        // And a qualitative one.
        let q = StructuredMeasure {
            item: "salt".into(),
            amount: Some(Amount::Qualitative {
                text: "to taste".into(),
            }),
            preparation: None,
            note: None,
        };
        let back: StructuredMeasure =
            serde_json::from_str(&serde_json::to_string(&q).unwrap()).unwrap();
        assert_eq!(q, back);
    }

    #[test]
    fn scaling_a_whole_structured_measure_leaves_prose_alone() {
        let m = StructuredMeasure {
            item: "flour".into(),
            amount: Some(amount(exact(2.0), "cup")),
            preparation: Some("sifted".into()),
            note: None,
        };
        let doubled = m.scaled(2.0);
        assert_eq!(doubled.preparation.as_deref(), Some("sifted"));
        if let Some(Amount::Quantified { quantity, .. }) = doubled.amount {
            assert_eq!(quantity, exact(4.0));
        } else {
            panic!("expected a quantified amount");
        }
    }
}
