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

use std::collections::HashMap;

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

/// The critical-path duration of a step DAG, in whole seconds — the recipe's total
/// time start to finish, **prep included** (#79). Prep steps are just nodes on the
/// graph, so "including prep" is free.
///
/// **Not** the sum of every step's `seconds`: parallel branches overlap, so the time
/// a cook actually spends is the *longest* dependency chain by summed duration — "chop
/// the onion (2 min) *while* the oil heats (3 min)" costs 3 min, not 5. Reading that
/// off a graph rather than a flat list is exactly why the DAG (#75) was worth building.
///
/// One forward pass, no arithmetic the model does: `finish(step) = max(finish of its
/// `after` deps) + step.seconds`, and the total is the greatest finish. The list is
/// its own topological sort ([`validate`] pins ids to `0..len` with `after` edges
/// pointing strictly earlier), so every dependency's finish is known before the step
/// that waits on it. A `finish` map keyed by id (not position) plus reading only deps
/// already seen means a malformed, out-of-order list degrades to a smaller estimate
/// rather than panicking — degrade-not-die, the same defence `stepDepths` takes.
///
/// Returns `None` when the number would be meaningless rather than a wrong one:
/// **no steps**, or **not one step is timed** (every `seconds` is `None`), so there is
/// no timing signal to sum. A graph with at least one timed step yields `Some`, and
/// that total is a **lower bound** when some steps are untimed — an untimed step adds
/// 0 to the running total but still carries its predecessors' time forward, so it can
/// never inflate the estimate. Accumulation is in `u64` and clamped back to `u32`, so
/// a pathological chain cannot overflow.
pub fn total_seconds(steps: &[StructuredStep]) -> Option<u32> {
    // Empty (vacuously all-none) and fully-untimed both fall here: no signal to sum.
    if steps.iter().all(|s| s.seconds.is_none()) {
        return None;
    }

    let mut finish: HashMap<u32, u64> = HashMap::with_capacity(steps.len());
    let mut total: u64 = 0;
    for step in steps {
        // `after` points only at earlier steps (validated), so their finishes are
        // already in the map; an unknown id (malformed input) is treated as a root.
        let deps_done = step
            .after
            .iter()
            .filter_map(|dep| finish.get(dep).copied())
            .max()
            .unwrap_or(0);
        let f = deps_done + u64::from(step.seconds.unwrap_or(0));
        finish.insert(step.id, f);
        total = total.max(f);
    }
    Some(total.min(u64::from(u32::MAX)) as u32)
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

    /// A straight chain has no parallelism, so its total is the plain sum of the
    /// durations along it — the degenerate case the critical path must still get right.
    #[test]
    fn total_of_a_linear_chain_is_the_sum() {
        let steps = vec![
            step(0, StepKind::Prep, Some(120), &[]),
            step(1, StepKind::Cook, Some(300), &[0]),
            step(2, StepKind::Cook, Some(600), &[1]),
        ];
        assert!(validate(&steps).is_ok());
        assert_eq!(total_seconds(&steps), Some(1020));
    }

    /// A diamond: one root feeds two parallel branches that rejoin. The total is the
    /// **longer** branch plus the shared head and tail — strictly less than summing
    /// every step, which is the whole point of reading a DAG rather than a flat list.
    #[test]
    fn total_of_a_diamond_is_the_critical_path_not_the_naive_sum() {
        // 0 (60) -> {1 (120), 2 (300)} -> 3 (30). Longest path is 0->2->3 = 390.
        let steps = vec![
            step(0, StepKind::Prep, Some(60), &[]),
            step(1, StepKind::Prep, Some(120), &[0]),
            step(2, StepKind::Cook, Some(300), &[0]),
            step(3, StepKind::Cook, Some(30), &[1, 2]),
        ];
        assert!(validate(&steps).is_ok());
        let naive_sum: u32 = steps.iter().filter_map(|s| s.seconds).sum();
        assert_eq!(
            naive_sum, 510,
            "the flat sum double-counts the parallel branches"
        );
        assert_eq!(
            total_seconds(&steps),
            Some(390),
            "critical path overlaps the shorter branch with the longer one"
        );
    }

    /// Two independent parallel roots (no edges between them): the total is the longer
    /// of the two, never their sum — "chop while the oil heats".
    #[test]
    fn total_of_independent_parallel_roots_is_the_longest() {
        let steps = vec![
            step(0, StepKind::Prep, Some(120), &[]), // chop the onion
            step(1, StepKind::Cook, Some(180), &[]), // heat the oil, meanwhile
        ];
        assert_eq!(total_seconds(&steps), Some(180));
    }

    /// Untimed steps make the estimate a lower bound: they carry a predecessor's time
    /// forward but add nothing, so they neither inflate nor blank a total that has at
    /// least one timed step.
    #[test]
    fn untimed_steps_contribute_zero_but_carry_predecessors_forward() {
        let steps = vec![
            step(0, StepKind::Prep, Some(300), &[]),
            step(1, StepKind::Cook, None, &[0]), // "until golden" — no timer
            step(2, StepKind::Cook, Some(60), &[1]),
        ];
        assert_eq!(total_seconds(&steps), Some(360));
    }

    /// Degenerate: no steps at all yields no estimate — absence, not a wrong `0`.
    #[test]
    fn total_of_no_steps_is_none() {
        assert_eq!(total_seconds(&[]), None);
    }

    /// Degenerate: a real graph where nothing is timed has no timing signal to sum, so
    /// the total is `None` (a lower bound of 0 would read as "instant", which is worse
    /// than admitting we don't know).
    #[test]
    fn total_of_a_fully_untimed_graph_is_none() {
        let steps = vec![
            step(0, StepKind::Prep, None, &[]),
            step(1, StepKind::Cook, None, &[0]),
        ];
        assert_eq!(total_seconds(&steps), None);
    }

    /// A single timed step is enough to yield an estimate.
    #[test]
    fn total_of_a_single_timed_step_is_that_duration() {
        assert_eq!(
            total_seconds(&[step(0, StepKind::Cook, Some(45), &[])]),
            Some(45)
        );
    }
}
