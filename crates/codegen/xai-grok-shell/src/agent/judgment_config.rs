//! `[judgment]` resolution for the session-scoped Jev hook.
//!
//! Mirrors the `auto_mode` resolver: the section is read from the effective config layers rather
//! than threaded through `spawn_session_actor`'s ~90-argument signature. A judgment section is a
//! user config file, not a per-session decision.

use xai_grok_config::JudgmentConfig;

/// Parse `[judgment]` out of one TOML document.
/// `None` when the table is absent, so a missing section stays distinguishable from a present but
/// empty one (both disable judgment, but only the absent case is "the user never opted in").
///
/// # Scope of the fail-open behavior below
///
/// This function is fail-open: a malformed table here degrades to vanilla behavior instead of
/// erroring. That is *not* the whole story for a malformed `[judgment]` section, because the typed
/// `Config` parse runs earlier and is strict — a type error in the table (for example
/// `enabled = "yes"`) aborts agent-config creation with `Error: Failed to create agent config`
/// and the session never starts. Malformed values of the *right* type that only fail here
/// (unknown shapes the typed parse tolerates) are what this branch actually catches.
fn judgment_config_from_toml(value: &toml::Value) -> Option<JudgmentConfig> {
    let section = value.get("judgment")?.clone();
    match section.try_into::<JudgmentConfig>() {
        Ok(config) => Some(config),
        Err(e) => {
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
        assert_eq!(config.timeout_ms, 1500);
    }

    /// A table this parser cannot read degrades to `None` rather than erroring.
    ///
    /// Note the scope: this exercises `judgment_config_from_toml` in isolation. In a real session
    /// the strict typed `Config` parse runs first, so a *type* error (the `enabled = "yes please"`
    /// case below) aborts agent-config creation outright — see the doc comment on
    /// `judgment_config_from_toml`. What this test pins is the parser's own contract.
    #[test]
    fn a_malformed_section_degrades_to_none_in_the_parser() {
        let value: toml::Value = toml::from_str("[judgment]\nenabled = \"yes please\"\n").unwrap();
        assert!(judgment_config_from_toml(&value).is_none());
    }

    /// The disk path is what real sessions use, and it is the seam the S1/S2 session tests
    /// deliberately bypass by injecting a hook. Cover it here so a regression in
    /// `ConfigLayers`/`GROK_HOME` resolution cannot silently disable every subsystem: with a
    /// `[judgment]` section on disk the hook must resolve, and with the section absent it must not.
    ///
    /// `serial` because it sets process-global `GROK_HOME`; `xai_dirs::grok_home()` is a `OnceLock`,
    /// so whichever test initializes it first wins.
    #[test]
    #[serial_test::serial]
    fn disk_section_resolves_to_a_hook_and_absence_does_not() {
        let home = tempfile::tempdir().unwrap();

        std::fs::write(
            home.path().join("config.toml"),
            "[judgment]\nenabled = true\napi_key = \"sk-test\"\ndynamic_thinking = true\n",
        )
        .unwrap();
        unsafe { std::env::set_var("GROK_HOME", home.path()) };
        let _ = xai_dirs::grok_home();

        let config = judgment_config_from_disk().expect("disk section parses");
        assert!(config.enabled, "enabled survives the disk round trip");
        assert_eq!(config.timeout_ms, 1500, "schema default applies over disk");
        assert!(
            resolve_judgment_hook().is_some(),
            "an enabled, keyed section on disk must produce a hook"
        );

        // Same home, section absent: judgment is off, and nothing is built.
        std::fs::write(home.path().join("config.toml"), "[cli]\nshow_tips = true\n").unwrap();
        assert!(judgment_config_from_disk().is_none());
        assert!(
            resolve_judgment_hook().is_none(),
            "no section on disk must not produce a hook"
        );
    }

    /// `enabled = false` on disk is the zero-regression path: the section parses but no hook is
    /// built, so no subsystem does any work.
    #[test]
    #[serial_test::serial]
    fn disabled_section_parses_but_yields_no_hook() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "[judgment]\nenabled = false\napi_key = \"sk-test\"\n",
        )
        .unwrap();
        unsafe { std::env::set_var("GROK_HOME", home.path()) };

        let config = judgment_config_from_disk().expect("section still parses");
        assert!(!config.enabled);
        assert!(resolve_judgment_hook().is_none(), "disabled ⇒ no hook");
    }
}
