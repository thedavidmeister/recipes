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

use std::collections::BTreeSet;

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
