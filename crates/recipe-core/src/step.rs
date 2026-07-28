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
//! **Every step carries a duration** (#158). A source that never says how long to
//! chop an onion has not described a timeless action — it has left a number out, and
//! the gap is in our reading, not in the dish. So the reading *estimates* what the
//! source leaves unstated, and [`StructuredStep::estimated`] records which of the two
//! kinds of claim a number is, so a display can be exactly as confident as the
//! reading deserves. [`validate`] refuses an untimed step, the way the equipment
//! reading refuses an empty one (#81): "this takes no time" is never true.
//!
//! A capture, not a derivation — the model is non-deterministic, so like a
//! [`StructuredMeasure`](crate::StructuredMeasure) reading this is a point-in-time
//! artifact, kept rather than re-extracted. No arithmetic lives here: the model
//! reads and estimates, [`total_seconds`] sums, and it knows nothing about where a
//! duration came from.

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
    /// How long the step takes, in whole seconds (#74). [`validate`] requires one on
    /// every step (#158), so a reading the app accepts has no holes in it.
    ///
    /// `Option` survives only for the readings taken before #158, when a source that
    /// stated no duration produced `None` and every such step contributed 0 to the
    /// critical path. Those rows still deserialize and still render; they are the
    /// thing a deliberate re-read replaces.
    pub seconds: Option<u32>,
    /// Whether `seconds` is the model's estimate rather than the source's own number
    /// (#158).
    ///
    /// `false` — the source stated it ("simmer for 20 minutes"). `true` — the source
    /// said nothing and the model put a cook's number on the action ("chop the
    /// onion", "fry until golden"). Both are real durations and both feed the
    /// critical path identically; they differ in how much a display should trust
    /// them, which is the whole reason to keep them apart.
    ///
    /// A flag on the number rather than a `Stated | Estimated` enum beside it,
    /// because it is a property *of* the duration: with no duration there is nothing
    /// to characterise, and `false` is then simply vacuous — where a `"stated"` tag
    /// on a step with no time would assert something untrue. It also defaults
    /// correctly for the readings already stored: those durations are all the
    /// source's own, since the prompt that produced them forbade inventing a timer.
    #[serde(default)]
    pub estimated: bool,
    /// Ids of the steps that must finish before this one begins (#75). Empty means
    /// no predecessor — a root the cook can start right away.
    #[serde(default)]
    pub after: Vec<u32>,
}

/// Check a step list is a well-formed, fully timed DAG in topological order.
///
/// The invariant is deliberately strict so the capture stays a valid graph and the
/// GUI can trust it. The push rejects a reading that violates any of it (the model is
/// re-run next pull), exactly as the ingredient push rejects a count mismatch:
///
/// - **Ids are `0..len` in order, and every `after` edge points to a *strictly
///   earlier* step.** That makes the list its own topological sort and rules out
///   cycles by construction — a step can only wait on steps already listed above it.
/// - **Every step carries a `seconds`** (#158). A step with no duration is not a
///   timeless step, it is an unfinished reading: it silently contributes 0 to the
///   critical path, which is why 13% of the corpus's stored estimates once claimed
///   under ten minutes for a recipe of a dozen steps. The source stating no number is
///   not an excuse to hold none — the reading estimates it and flags it
///   [`estimated`](StructuredStep::estimated). Refusing the hole here is the same
///   ruling the equipment reading makes against an empty list (#81): a recipe that
///   "needs nothing" and a step that "takes no time" are equally never true.
///
/// Only the push is gated. Readings stored before #158 keep untimed steps and still
/// deserialize, render and total — [`validate`] is not run on load, so nothing
/// already captured is invalidated by tightening what we will newly accept.
pub fn validate(steps: &[StructuredStep]) -> Result<(), String> {
    for (i, step) in steps.iter().enumerate() {
        if step.id != i as u32 {
            return Err(format!(
                "step at position {i} has id {} — ids must be 0-based and sequential",
                step.id
            ));
        }
        if step.seconds.is_none() {
            return Err(format!(
                "step {} has no duration — every step takes some time, so estimate it \
                 and set `estimated` rather than leaving it unread",
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
/// no timing signal to sum. A graph with at least one timed step yields `Some`.
///
/// **Estimated durations are summed exactly like stated ones.** This function does not
/// read [`StructuredStep::estimated`] and has no arithmetic to change on its account —
/// the model estimates, code sums, and #158 changed only what the model hands over.
/// What it changed is the *input*: a reading [`validate`] accepts has a duration on
/// every step, so there are no zeros left to swallow. On such a graph the total is a
/// genuine estimate of the whole cook. On a reading stored before #158 it is still a
/// **lower bound** — an untimed step adds 0 while carrying its predecessors' time
/// forward, so it can never inflate the total, only understate it. Accumulation is in
/// `u64` and clamped back to `u32`, so a pathological chain cannot overflow.
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

    /// A step whose duration the source stated.
    fn step(id: u32, kind: StepKind, seconds: Option<u32>, after: &[u32]) -> StructuredStep {
        StructuredStep {
            id,
            text: format!("step {id}"),
            kind,
            seconds,
            estimated: false,
            after: after.to_vec(),
        }
    }

    /// A step whose duration the model estimated because the source stated none.
    fn guess(id: u32, kind: StepKind, seconds: u32, after: &[u32]) -> StructuredStep {
        StructuredStep {
            estimated: true,
            ..step(id, kind, Some(seconds), after)
        }
    }

    #[test]
    fn round_trips_with_snake_case_kind_and_edges() {
        let steps = vec![
            guess(0, StepKind::Prep, 90, &[]),
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
        assert!(json.contains(r#""estimated":true"#), "the guess is marked");
        assert!(
            json.contains(r#""estimated":false"#),
            "the stated one is marked too — the flag is always written, so a reader \
             never has to infer it from an absent key: {json}"
        );
        let back: Vec<StructuredStep> = serde_json::from_str(&json).unwrap();
        assert_eq!(steps, back);
    }

    /// Every reading stored before #158 lacks the `estimated` key and may carry a null
    /// `seconds`. Both must keep deserializing — the corpus holds 790 such readings,
    /// and a shape change that dropped them would blank every step in the app until a
    /// re-read finished.
    #[test]
    fn a_reading_stored_before_the_estimate_flag_still_deserializes() {
        let s: StructuredStep =
            serde_json::from_str(r#"{"id":0,"text":"chop","kind":"prep","seconds":null}"#).unwrap();
        assert_eq!(s.seconds, None);
        assert!(s.after.is_empty(), "`after` may be omitted entirely");
        assert!(
            !s.estimated,
            "an absent flag means stated: the prompt that produced these readings \
             forbade inventing a timer, so every number in them is the source's own"
        );

        let timed: StructuredStep = serde_json::from_str(
            r#"{"id":0,"text":"simmer","kind":"cook","seconds":1200,"after":[]}"#,
        )
        .unwrap();
        assert_eq!(timed.seconds, Some(1200));
        assert!(!timed.estimated);
    }

    #[test]
    fn validate_accepts_a_topologically_ordered_dag() {
        // Two parallel prep roots, then a cook step that waits on both. The prep
        // durations are the model's estimates; the cook step's came from the source.
        let steps = vec![
            guess(0, StepKind::Prep, 60, &[]),
            guess(1, StepKind::Prep, 45, &[]),
            step(2, StepKind::Cook, Some(120), &[0, 1]),
        ];
        assert!(validate(&steps).is_ok());
        assert!(validate(&[]).is_ok());
    }

    /// The #158 gate: a step with no duration is an unfinished reading, not a timeless
    /// action, so the push refuses it — whether or not the rest of the graph is sound.
    #[test]
    fn validate_rejects_a_step_with_no_duration() {
        let err = validate(&[
            guess(0, StepKind::Prep, 60, &[]),
            step(1, StepKind::Cook, None, &[0]),
        ])
        .unwrap_err();
        assert!(err.contains("step 1"), "names the offending step: {err}");
        assert!(err.contains("no duration"), "{err}");

        // Even one step, alone and otherwise well-formed, is not enough.
        assert!(validate(&[step(0, StepKind::Cook, None, &[])]).is_err());
    }

    /// An estimate is a real duration, so a fully estimated reading is accepted — the
    /// gate is against holding *no* number, never against the model supplying one.
    #[test]
    fn validate_accepts_a_wholly_estimated_reading() {
        let steps = vec![
            guess(0, StepKind::Prep, 90, &[]),
            guess(1, StepKind::Cook, 300, &[0]),
            guess(2, StepKind::Cook, 600, &[1]),
        ];
        assert!(validate(&steps).is_ok());
    }

    #[test]
    fn validate_rejects_non_sequential_ids() {
        let steps = vec![
            guess(0, StepKind::Prep, 60, &[]),
            guess(2, StepKind::Cook, 60, &[0]),
        ];
        assert!(validate(&steps).is_err());
    }

    #[test]
    fn validate_rejects_a_forward_or_self_dependency() {
        // A step depending on itself or a later step would allow a cycle.
        assert!(validate(&[guess(0, StepKind::Cook, 60, &[0])]).is_err());
        let forward = vec![
            guess(0, StepKind::Cook, 60, &[1]),
            guess(1, StepKind::Cook, 60, &[]),
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

    /// The #158 shape: a recipe whose source states no time anywhere still yields a
    /// total, because the reading estimated every step. This is the case that used to
    /// return `None` and leave the recipe among the 77 with no number at all.
    #[test]
    fn total_of_a_wholly_estimated_reading_is_a_real_total() {
        let steps = vec![
            guess(0, StepKind::Prep, 120, &[]),    // "slice the cucumber"
            guess(1, StepKind::Cook, 300, &[]),    // "cook the noodles per pack"
            guess(2, StepKind::Cook, 60, &[0, 1]), // "mix in a serving dish"
        ];
        assert!(validate(&steps).is_ok());
        assert_eq!(
            total_seconds(&steps),
            Some(360),
            "the critical path runs through the longer branch, exactly as it would \
             for stated durations"
        );
    }

    /// `total_seconds` is blind to provenance: two identical graphs, one stated and one
    /// estimated, produce the same number. The split #158 preserves is that the model
    /// supplies durations and this code only sums them — a total that changed because a
    /// number was a guess would be arithmetic leaking into the capture.
    #[test]
    fn the_total_does_not_depend_on_where_a_duration_came_from() {
        let stated = vec![
            step(0, StepKind::Prep, Some(120), &[]),
            step(1, StepKind::Cook, Some(300), &[0]),
        ];
        let estimated = vec![
            guess(0, StepKind::Prep, 120, &[]),
            guess(1, StepKind::Cook, 300, &[0]),
        ];
        assert_eq!(total_seconds(&stated), total_seconds(&estimated));
        assert_eq!(total_seconds(&estimated), Some(420));
    }
}
