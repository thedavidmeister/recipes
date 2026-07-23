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
}
