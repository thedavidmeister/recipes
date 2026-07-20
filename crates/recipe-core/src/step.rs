//! Structured cooking steps (#74/#75/#76): a model's reading of a recipe's method
//! into a DAG.
//!
//! The instructions arrive as one prose blob. The step-reading enrichment segments
//! them into [`StructuredStep`]s and maps the dependencies between them, so the GUI
//! renders a graph rather than a newline split: a timer rides on
//! [`StructuredStep::seconds`] (#74), parallel-vs-sequential is derived from
//! [`StructuredStep::after`] (#75), and prep pulled out of an ingredient line
//! ("100g chopped onions") lands as a [`StepKind::Prep`] step with no predecessors
//! (#76).
//!
//! A capture, not a derivation — the model is non-deterministic, so like a
//! [`StructuredMeasure`](crate::StructuredMeasure) reading this is a point-in-time
//! artifact, kept rather than re-extracted. No arithmetic lives here.

use serde::{Deserialize, Serialize};

/// Whether a step is mise en place or active cooking. Prep steps — including prep
/// extracted from an ingredient's preparation (#76) — tend to be parallelizable
/// roots; cook steps carry the sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Prep,
    Cook,
}

/// One node in a recipe's method DAG.
///
/// `id` is 0-based and stable within the recipe; `after` holds the ids of the steps
/// that must complete before this one begins (`[]` = can start immediately). The
/// ordering *is* those edges — parallel vs sequential is read off the graph, never
/// stored separately, so there is one source of truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredStep {
    pub id: u32,
    pub text: String,
    pub kind: StepKind,
    /// A timer's duration in whole seconds, when the step is timed (#74); `None`
    /// for an untimed step ("until golden").
    pub seconds: Option<u32>,
    /// Ids of the steps that must finish before this one begins (#75). Empty means
    /// no predecessor — a root the cook can start right away.
    #[serde(default)]
    pub after: Vec<u32>,
}

/// Check a step list is a well-formed DAG in topological order.
///
/// The invariant is deliberately strict so the capture stays a valid graph and the
/// GUI can trust it: ids are `0..len` in order, and every `after` edge points to a
/// *strictly earlier* step. That makes the list its own topological sort and rules
/// out cycles by construction — a step can only wait on steps already listed above
/// it. The push rejects a reading that violates this (the model is re-run next
/// pull), exactly as the ingredient push rejects a count mismatch.
pub fn validate(steps: &[StructuredStep]) -> Result<(), String> {
    for (i, step) in steps.iter().enumerate() {
        if step.id != i as u32 {
            return Err(format!(
                "step at position {i} has id {} — ids must be 0-based and sequential",
                step.id
            ));
        }
        for &dep in &step.after {
            if dep >= step.id {
                return Err(format!(
                    "step {} depends on {dep}, which is not an earlier step",
                    step.id
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: u32, kind: StepKind, seconds: Option<u32>, after: &[u32]) -> StructuredStep {
        StructuredStep {
            id,
            text: format!("step {id}"),
            kind,
            seconds,
            after: after.to_vec(),
        }
    }

    #[test]
    fn round_trips_with_snake_case_kind_and_edges() {
        let steps = vec![
            step(0, StepKind::Prep, None, &[]),
            step(1, StepKind::Cook, Some(1800), &[0]),
        ];
        let json = serde_json::to_string(&steps).unwrap();
        assert!(
            json.contains(r#""kind":"prep""#),
            "kind is snake_case: {json}"
        );
        assert!(json.contains(r#""kind":"cook""#));
        assert!(json.contains(r#""seconds":1800"#));
        assert!(json.contains(r#""after":[0]"#));
        let back: Vec<StructuredStep> = serde_json::from_str(&json).unwrap();
        assert_eq!(steps, back);
    }

    #[test]
    fn a_null_seconds_and_absent_after_deserialize() {
        // The untimed, no-predecessor case — `after` may be omitted entirely.
        let s: StructuredStep =
            serde_json::from_str(r#"{"id":0,"text":"chop","kind":"prep","seconds":null}"#).unwrap();
        assert_eq!(s.seconds, None);
        assert!(s.after.is_empty());
    }

    #[test]
    fn validate_accepts_a_topologically_ordered_dag() {
        // Two parallel prep roots, then a cook step that waits on both.
        let steps = vec![
            step(0, StepKind::Prep, None, &[]),
            step(1, StepKind::Prep, None, &[]),
            step(2, StepKind::Cook, Some(120), &[0, 1]),
        ];
        assert!(validate(&steps).is_ok());
        assert!(validate(&[]).is_ok());
    }

    #[test]
    fn validate_rejects_non_sequential_ids() {
        let steps = vec![
            step(0, StepKind::Prep, None, &[]),
            step(2, StepKind::Cook, None, &[0]),
        ];
        assert!(validate(&steps).is_err());
    }

    #[test]
    fn validate_rejects_a_forward_or_self_dependency() {
        // A step depending on itself or a later step would allow a cycle.
        assert!(validate(&[step(0, StepKind::Cook, None, &[0])]).is_err());
        let forward = vec![
            step(0, StepKind::Cook, None, &[1]),
            step(1, StepKind::Cook, None, &[]),
        ];
        assert!(validate(&forward).is_err());
    }
}
