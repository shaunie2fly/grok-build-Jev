//! High-level Jev judgments: one method per token/execution subsystem.
//!
//! Every method is fail-*open toward vanilla behavior*: an unreachable endpoint, a timeout, a
//! rejected key, or a malformed body yields the caller's pre-Jev default rather than an error.
//! That keeps the "zero-regression" guarantee independent of the judgment service's health.

use super::client::JevClient;
use serde_json::{Value, json};
use tracing::warn;
use xai_grok_sampling_types::ReasoningEffort;

/// Ordered difficulty levels for [`JudgmentEvaluator::classify_reasoning`].
/// Jev scores a `score` question as the probability-weighted mean of these indices, so
/// [`SCORE_MAX_INDEX`] is what `score / SCORE_MAX_INDEX` normalizes against.
const REASONING_LEVELS: [&str; 4] = [
    "Trivial: a lookup, a rename, or a one-line syntax fix with no design choice",
    "Simple: a localized edit in one file that follows an existing pattern",
    "Moderate: a multi-file change needing design choices or careful integration",
    "Complex: deep algorithmic reasoning, subtle concurrency, or an architectural refactor",
];

/// Ordered quality levels for [`JudgmentEvaluator::score_patch`].
const PATCH_LEVELS: [&str; 4] = [
    "Incomplete or wrong: misses the task requirements or introduces a regression",
    "Partial: addresses the task but leaves stubs, gaps, or untested edge cases",
    "Good: meets the requirements with only minor style or coverage gaps",
    "Excellent: completely and cleanly addresses the task with no regressions",
];

/// Highest level index shared by [`REASONING_LEVELS`] and [`PATCH_LEVELS`].
const SCORE_MAX_INDEX: f64 = 3.0;

/// Character caps on what is sent as `state`, keeping a single judgment request small.
const MAX_OUTPUT_SAMPLE_CHARS: usize = 2500;
const MAX_DIFF_CHARS: usize = 8000;

/// Evaluates Jev judgments for the token-optimization subsystems.
#[derive(Clone)]
pub struct JudgmentEvaluator {
    client: JevClient,
    safety_threshold: f32,
}

impl JudgmentEvaluator {
    pub fn new(client: JevClient, safety_threshold: f32) -> Self {
        Self {
            client,
            safety_threshold,
        }
    }

    /// Subsystem 1: Dynamic Reasoning Classification.
    /// Returns one of `offered` (the active model's thinking menu) for a prompt (`Some`).
    /// Bands are equal-width over that list, cheapest first. An empty `offered` (model has no
    /// thinking settings) yields `None` with no network call. `"minimal"` is only emitted if the
    /// model actually offers it.
    /// `None` also means Jev was unavailable or returned nothing usable: the caller must keep the
    /// already-configured effort rather than substituting a fabricated level.
    pub async fn classify_reasoning(
        &self,
        prompt: &str,
        subagent_type: Option<&str>,
        offered: &[ReasoningEffort],
    ) -> Option<&'static str> {
        if offered.is_empty() {
            return None;
        }
        // An explore subagent only reads and reports, so its effort is the cheapest menu row
        // without a call.
        if let Some("explore") = subagent_type {
            return cheapest_offered_effort(offered);
        }

        let state = json!({ "prompt": prompt, "subagent_type": subagent_type });
        let questions = json!({
            "complexity": {
                "type": "score",
                "instructions": "Evaluate the cognitive difficulty of this task.",
                "criteria": REASONING_LEVELS,
            }
        });

        match self.client.evaluate(state, questions).await {
            Ok(response) => match normalized_score(&response, "complexity") {
                Some(score) => map_score_onto_offered_efforts(score, offered),
                None => {
                    warn!(
                        "Jev reasoning classification returned no usable score; keeping configured effort"
                    );
                    None
                }
            },
            Err(e) => {
                warn!("Jev dynamic thinking evaluation failed, keeping configured effort: {e}");
                None
            }
        }
    }

    /// Subsystem 2: Output Distillation Check.
    /// `true` means the output carries something a developer must act on, so it must be kept in
    /// full. Fails open to `true`: keeping output is always safe, distilling it is not.
    pub async fn has_actionable_errors(&self, output_sample: &str) -> bool {
        let state = json!({ "output_sample": truncate(output_sample, MAX_OUTPUT_SAMPLE_CHARS) });
        let questions = json!({
            "has_errors": {
                "type": "noul",
                "instructions": "Does this CLI/test output contain active compiler errors, stack traces, or failed unit assertions requiring developer action?",
            }
        });

        match self.client.evaluate(state, questions).await {
            // `noul` is the probability of "yes"; anything above a coin flip counts as actionable.
            Ok(response) => answer_number(&response, "has_errors", "noul").unwrap_or(1.0) > 0.50,
            Err(e) => {
                warn!("Jev output distillation check failed, failing open: {e}");
                true
            }
        }
    }

    /// Subsystem 3: Tournament Patch Quality Scorer.
    /// Returns `0.0`-`1.0`, higher is better; `0.5` is the neutral score when Jev is unavailable,
    /// so a failed call cannot be mistaken for a strong or a disqualified candidate.
    pub async fn score_patch(&self, task: &str, diff: &str) -> f64 {
        let state = json!({
            "task": task,
            "diff": truncate(diff, MAX_DIFF_CHARS),
        });
        let questions = json!({
            "quality": {
                "type": "score",
                "instructions": "Score how cleanly and completely this git patch addresses the task requirements without regressions or incomplete stubs.",
                "criteria": PATCH_LEVELS,
            }
        });

        match self.client.evaluate(state, questions).await {
            Ok(response) => normalized_score(&response, "quality").unwrap_or(0.0),
            Err(e) => {
                warn!("Jev patch scoring failed: {e}");
                0.5
            }
        }
    }

    /// Subsystem 4: Dead-End Verification.
    /// `true` only on a confident "this was abandoned and fully reverted" verdict. Every failure
    /// mode returns `false`, so pruning never happens without a definitive positive answer.
    pub async fn verify_dead_end(&self, summary: &str) -> bool {
        let state = json!({ "turn_summary": summary });
        let questions = json!({
            "is_abandoned": {
                "type": "noul",
                "instructions": "Did this series of turns represent an abandoned debugging hypothesis that was fully reverted and provides no forward utility?",
            }
        });

        match self.client.evaluate(state, questions).await {
            Ok(response) => answer_number(&response, "is_abandoned", "noul").unwrap_or(0.0) > 0.75,
            Err(_) => false,
        }
    }

    /// Subsystem 5: Pre-Execution Safety Gating.
    /// `true` means Jev scored the call as non-destructive, letting it skip the approval prompt.
    /// Fails closed to `false`: a destructive call must always reach the user.
    pub async fn is_safe_tool(&self, tool: &str, input: &str) -> bool {
        let state = json!({ "tool": tool, "input": input });
        let questions = json!({
            "is_destructive": {
                "type": "noul",
                "instructions": "Is this command or edit destructive, irreversible, dropping state, or deleting uncommitted work outside the workspace?",
            }
        });

        match self.client.evaluate(state, questions).await {
            // `is_destructive` is the probability of "yes"; safe means below the risk threshold.
            Ok(response) => match answer_number(&response, "is_destructive", "noul") {
                Some(risk) => risk < f64::from(self.safety_threshold),
                None => false,
            },
            Err(_) => false,
        }
    }
}

/// Read `answers.<question>.<field>` as a number.
/// Returns `None` for any missing or wrongly-typed hop, which callers map to their fallback.
fn answer_number(response: &Value, question: &str, field: &str) -> Option<f64> {
    response.get("answers")?.get(question)?.get(field)?.as_f64()
}

/// Intensity rank for sorting a model's menu cheapest-first.
fn effort_intensity(effort: ReasoningEffort) -> u8 {
    match effort {
        ReasoningEffort::None => 0,
        ReasoningEffort::Minimal => 1,
        ReasoningEffort::Low => 2,
        ReasoningEffort::Medium => 3,
        ReasoningEffort::High => 4,
        ReasoningEffort::Xhigh => 5,
        ReasoningEffort::Max => 6,
    }
}

fn ordered_offered_efforts(offered: &[ReasoningEffort]) -> Vec<ReasoningEffort> {
    let mut ordered = offered.to_vec();
    ordered.sort_by_key(|effort| effort_intensity(*effort));
    ordered.dedup();
    ordered
}

/// Cheapest row on the model's thinking menu. `None` when the model offers no thinking settings.
pub(crate) fn cheapest_offered_effort(offered: &[ReasoningEffort]) -> Option<&'static str> {
    ordered_offered_efforts(offered)
        .first()
        .copied()
        .map(Into::into)
}

/// Map a 0.0–1.0 complexity score onto `offered`, cheapest-first, equal-width bands.
/// Score 1.0 lands on the strongest offered level. Empty `offered` yields `None`.
pub(crate) fn map_score_onto_offered_efforts(
    score: f64,
    offered: &[ReasoningEffort],
) -> Option<&'static str> {
    let ordered = ordered_offered_efforts(offered);
    if ordered.is_empty() {
        return None;
    }
    let n = ordered.len();
    let idx = ((score.clamp(0.0, 1.0) * n as f64) as usize).min(n - 1);
    ordered.get(idx).copied().map(Into::into)
}

/// Read a `score` answer and normalize it onto `0.0`-`1.0`.
/// A `score` question answers with the probability-weighted mean of its level indices, so the raw
/// value spans `0..=MAX_INDEX`; dividing by the maximum index puts it back on the `0.0` = trivial,
/// `1.0` = hardest scale the thresholds below are written against.
fn normalized_score(response: &Value, question: &str) -> Option<f64> {
    let raw = answer_number(response, question, "score")?;
    Some((raw / SCORE_MAX_INDEX).clamp(0.0, 1.0))
}

/// Truncate on a character boundary (never bytes, so multi-byte text cannot be split).
fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
#[path = "evaluator_tests.rs"]
mod tests;
