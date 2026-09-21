//! `[judgment]` resolution for the session-scoped Jev hook.
//!
//! Mirrors the `auto_mode` resolver: the section is read from the effective config layers rather
//! than threaded through `spawn_session_actor`'s ~90-argument signature. A judgment section is a
//! user config file, not a per-session decision.

use xai_grok_config::JudgmentConfig;

/// Parse `[judgment]` out of one TOML document.
/// `None` when the table is absent, so a missing section stays distinguishable from a present but
/// empty one (both disable judgment, but only the absent case is "the user never opted in").
fn judgment_config_from_toml(value: &toml::Value) -> Option<JudgmentConfig> {
    let section = value.get("judgment")?.clone();
    match section.try_into::<JudgmentConfig>() {
        Ok(config) => Some(config),
        Err(e) => {
            // A malformed section must not fail the session: judgment is optional, so it degrades
            // to vanilla behavior like every other runtime failure in this subsystem.
            tracing::warn!(error = %e, "`[judgment]` is malformed; judgment stays disabled");
            None
        }
    }
}

/// The effective `[judgment]` section from disk, or `None` when absent/unreadable.
/// Overlay-free, matching `resolve_auto_mode_config_from_disk`: the `GROK_CONFIG` overlay is a
/// soft-settings channel and must not be able to switch on a feature that spends user credentials.
pub(crate) fn judgment_config_from_disk() -> Option<JudgmentConfig> {
    match crate::config::ConfigLayers::load() {
        Ok(layers) => judgment_config_from_toml(&layers.effective_config_base_without_overlay()),
        Err(e) => {
            tracing::warn!(error = %e, "judgment: failed to load config layers");
            None
        }
    }
}

/// Session and subagent spawn both resolve through this helper so they share one overlay-free
/// source of truth. A process-wide cache is deliberately not used: tests would leak, and
/// restart-required already means a session snapshots the hook on first use.
pub(crate) fn resolve_judgment_hook() -> Option<crate::judgment::JudgmentHook> {
    crate::judgment::JudgmentHook::from_config(judgment_config_from_disk().as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_section_yields_none() {
        let value: toml::Value = toml::from_str("[other]\nkey = 1\n").unwrap();
        assert!(judgment_config_from_toml(&value).is_none());
    }

    #[test]
    fn present_section_is_parsed_with_its_defaults() {
        let value: toml::Value =
            toml::from_str("[judgment]\nenabled = true\ndynamic_thinking = true\n").unwrap();
        let config = judgment_config_from_toml(&value).expect("section parses");
        assert!(config.enabled);
        assert!(config.dynamic_thinking);
        assert!(config.gate_tools, "unset UI lever defaults on");
        assert_eq!(config.endpoint, "https://api.typesafe.ai/v1/systemone");
        assert_eq!(config.timeout_ms, 400);
    }

    /// A typo in an optional section disables judgment; it must not take the session down.
    #[test]
    fn a_malformed_section_disables_judgment_instead_of_failing() {
        let value: toml::Value = toml::from_str("[judgment]\nenabled = \"yes please\"\n").unwrap();
        assert!(judgment_config_from_toml(&value).is_none());
    }
}
