//! Session-scoped judgment hook: resolves the `[judgment]` config and owns the lazy evaluator.
//!
//! The config is re-read from disk the same way [`crate::util::config::resolve_auto_mode_config_from_disk`]
//! does, rather than threaded through `spawn_session_actor`'s parameter list: that signature already
//! carries ~90 arguments, and a judgment section is a user config file, not a per-session decision.
//! The evaluator is built once, on first use, and only when a credential actually exists — an
//! unconfigured install pays one cheap `None` check per turn and never constructs an HTTP client.

use std::sync::Arc;

use xai_grok_config::JudgmentConfig;

use super::client::JevClient;
use super::evaluator::JudgmentEvaluator;

/// The resolved, session-usable judgment hook.
///
/// `None` on [`Self::from_config`] means every judgment subsystem is off: the caller keeps vanilla
/// behavior with no extra branches beyond an `Option` check.
#[derive(Clone)]
pub struct JudgmentHook {
    evaluator: Arc<JudgmentEvaluator>,
    config: JudgmentConfig,
}

impl JudgmentHook {
    /// Build a hook from a resolved `[judgment]` section, or `None` when it is unusable.
    ///
    /// Returns `None` when judgment is disabled or no credential resolves. A configured but
    /// unresolvable `env:VAR` also yields `None`, so a user who named a credential gets vanilla
    /// behavior rather than a silent fall back to an unrelated key.
    pub fn from_config(config: Option<&JudgmentConfig>) -> Option<Self> {
        let config = config?;
        if !config.enabled {
            return None;
        }
        let client = JevClient::try_new(
            config.api_key.clone(),
            config.endpoint.clone(),
            config.timeout_ms,
        )?;
        Some(Self {
            evaluator: Arc::new(JudgmentEvaluator::new(client, config.safety_threshold)),
            config: config.clone(),
        })
    }

    /// The evaluator, for the call sites that have already gated on a specific lever.
    pub fn evaluator(&self) -> &JudgmentEvaluator {
        &self.evaluator
    }

    /// The resolved config, so call sites can read their own feature lever.
    pub fn config(&self) -> &JudgmentConfig {
        &self.config
    }

    /// Whether dynamic parent-turn reasoning is on.
    pub fn dynamic_thinking_enabled(&self) -> bool {
        self.config.enabled && self.config.dynamic_thinking
    }

    /// Whether dynamic subagent reasoning is on.
    pub fn dynamic_subagent_thinking_enabled(&self) -> bool {
        self.config.enabled && self.config.dynamic_subagent_thinking
    }

    /// Whether tournament candidate pruning is on.
    pub fn tournament_pruning_enabled(&self) -> bool {
        self.config.enabled && self.config.tournament_pruning
    }

    /// Whether tool-output distillation is on.
    pub fn distill_outputs_enabled(&self) -> bool {
        self.config.enabled && self.config.distill_outputs
    }

    /// Whether contiguous-tail dead-end scrubbing is on.
    pub fn prune_dead_ends_enabled(&self) -> bool {
        self.config.enabled && self.config.prune_dead_ends
    }

    /// Whether pre-execution tool safety gating is on.
    pub fn gate_tools_enabled(&self) -> bool {
        self.config.enabled && self.config.gate_tools
    }

    /// The output line count above which distillation is considered.
    pub fn distill_line_threshold(&self) -> usize {
        self.config.distill_line_threshold
    }
}

#[cfg(test)]
#[path = "hook_tests.rs"]
mod tests;
