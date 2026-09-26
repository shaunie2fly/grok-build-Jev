//! `[judgment]` section: settings for the TypeSafe Jev judgment provider (sub-300 ms
//! non-generative System 1 classifications).

use serde::{Deserialize, Serialize};

/// `[judgment]` section: gates every Jev-backed token/execution optimization.
/// ONE struct serves the local `[judgment]` TOML table, so the runtime keeps vanilla behavior
/// whenever the section is absent or [`Self::enabled`] is `false`.
/// [`Self::api_key`] holds either a literal key or `env:VAR_NAME`; with neither set, the client
/// reads the `TYPESAFE_API_KEY` environment variable at construction.
/// Each field's serde default is the documented default, so a partial table (for example only
/// `enabled = true`) still resolves to a usable endpoint, timeout, and threshold.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct JudgmentConfig {
    /// Master switch for every Jev-backed optimization.
    #[serde(default)]
    pub enabled: bool,

    /// Explicit key or `env:TYPESAFE_API_KEY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Jev system-one endpoint.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,

    /// Per-call timeout in milliseconds; a Jev call that exceeds it falls back to default behavior.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    // 1. Dynamic Parent Reasoning
    /// Let Jev pick the parent turn's reasoning effort instead of the configured default.
    /// Defaults ON so `[judgment] enabled = true` (or a Settings master-switch toggle) produces
    /// the documented feature set without a second lever flip.
    #[serde(default = "default_true")]
    pub dynamic_thinking: bool,

    // 2. Dynamic Subagent Reasoning
    /// Let Jev pick the reasoning effort of spawned subagents.
    #[serde(default)]
    pub dynamic_subagent_thinking: bool,

    // 3. Subagent Tournament Pruning
    /// Score competing subagent patches with Jev and keep only the best candidate's transcript.
    #[serde(default)]
    pub tournament_pruning: bool,

    // 4. Output Distillation
    /// Replace large, error-free tool output with a head/tail summary plus a saved full log.
    #[serde(default = "default_true")]
    pub distill_outputs: bool,
    /// Output line count above which [`Self::distill_outputs`] considers distilling.
    #[serde(default = "default_distill_threshold")]
    pub distill_line_threshold: usize,

    // 5. Dead-End Tail Scrubbing
    /// Drop contiguous trailing turns that Jev confirms were a reverted dead end.
    #[serde(default)]
    pub prune_dead_ends: bool,

    // 6. Pre-Execution Tool Gating
    /// Let Jev auto-approve tool calls it scores as non-destructive.
    #[serde(default = "default_true")]
    pub gate_tools: bool,
    /// Risk score below which a gated tool call is treated as safe.
    #[serde(default = "default_safety_threshold")]
    pub safety_threshold: f32,
}

impl Default for JudgmentConfig {
    /// The same values the serde defaults apply, so a programmatically-built config cannot drift
    /// from a parsed one. A derived `Default` would yield an empty endpoint and `timeout_ms = 0`,
    /// which fails every Jev call before it starts.
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            endpoint: default_endpoint(),
            timeout_ms: default_timeout_ms(),
            dynamic_thinking: default_true(),
            dynamic_subagent_thinking: false,
            tournament_pruning: false,
            distill_outputs: default_true(),
            distill_line_threshold: default_distill_threshold(),
            prune_dead_ends: false,
            gate_tools: default_true(),
            safety_threshold: default_safety_threshold(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_endpoint() -> String {
    "https://api.typesafe.ai/v1/systemone".to_string()
}
fn default_timeout_ms() -> u64 {
    // Measured against the live Jev endpoint: p50 ~465 ms, p95 ~568 ms, p99 ~631 ms warm,
    // with cold-cache calls observed up to ~1170 ms. The original 400 ms budget expired on
    // every observed call (0/20 succeeded), which silently disabled every subsystem while
    // looking configured. 1500 ms clears the observed p99 and worst cold call with margin,
    // and costs only latency when the endpoint is genuinely unwell — every caller fails open.
    1500
}
fn default_distill_threshold() -> usize {
    40
}
fn default_safety_threshold() -> f32 {
    // `is_safe_tool` allows a call when Jev's destructive-probability (`noul`) is *below* this
    // value, so a higher threshold is more permissive.
    //
    // Measured separation on the live endpoint (n=12 per command): benign commands
    // (`git commit`, `cp`, `git add`, `npm install`, `cargo build`) score 0.02–0.07, while
    // destructive ones (`dd`, `chown -R`, `mv <repo>`, `chmod 000`, `git branch -D`) score
    // 0.10–0.59. The bands do not overlap; the gap is [0.07, 0.10].
    //
    // The previous 0.20 sat *above* the destructive floor, so it auto-approved genuinely
    // harmful calls — `dd if=/dev/zero of=…` scored 0.10 and `chown -R root:root /mnt/data`
    // scored 0.16. 0.08 sits inside the measured gap and biases toward asking.
    0.08
}

#[cfg(test)]
#[path = "judgment_tests.rs"]
mod tests;
