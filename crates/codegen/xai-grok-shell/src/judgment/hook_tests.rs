//! [`super::JudgmentHook`] gating tests.
//!
//! These construct configs directly. The endpoint is never contacted.
//! The credential-less case clears the process fallback variables for the duration of the test,
//! because a developer shell often exports them.

use super::*;
use serial_test::serial;
use xai_grok_test_support::EnvGuard;

const ENDPOINT: &str = "http://127.0.0.1:1/";

fn enabled() -> JudgmentConfig {
    JudgmentConfig {
        enabled: true,
        api_key: Some("sk-test".to_owned()),
        endpoint: ENDPOINT.to_owned(),
        ..JudgmentConfig::default()
    }
}

/// The zero-regression contract: an absent, disabled, or credential-less section yields no hook,
/// so every subsystem keeps vanilla behavior.
#[test]
#[serial]
fn no_hook_without_an_enabled_configured_section() {
    assert!(JudgmentHook::from_config(None).is_none(), "section absent");
    assert!(
        JudgmentHook::from_config(Some(&JudgmentConfig::default())).is_none(),
        "enabled defaults to false"
    );

    let without_key = JudgmentConfig {
        enabled: true,
        api_key: None,
        endpoint: ENDPOINT.to_owned(),
        ..JudgmentConfig::default()
    };
    // Fallback variables are part of credential resolution. Clear them so this assertion tests
    // the credential-less path rather than whatever the developer shell exported.
    let _primary = EnvGuard::unset("TYPESAFE_API_KEY");
    let _secondary = EnvGuard::unset("JEV_TYPESAFE_AI_KEY");
    assert!(
        super::super::client::JevClient::try_new(None, ENDPOINT.to_owned(), 400).is_none(),
        "with both fallback variables unset, a keyless config must not build a client"
    );
    assert!(JudgmentHook::from_config(Some(&without_key)).is_none());

    let named_missing = JudgmentConfig {
        api_key: Some("env:JEV_TEST_ABSENT_VAR".to_owned()),
        ..without_key
    };
    assert!(
        JudgmentHook::from_config(Some(&named_missing)).is_none(),
        "an explicitly named credential that does not resolve must fail closed"
    );
}

#[test]
fn an_enabled_configured_section_yields_a_hook() {
    let hook = JudgmentHook::from_config(Some(&enabled())).expect("enabled + keyed ⇒ hook");
    assert_eq!(hook.config().endpoint, ENDPOINT);
}

/// Each lever is independent: enabling one must not enable the others.
#[test]
fn each_lever_gates_on_its_own_flag() {
    let hook = JudgmentHook::from_config(Some(&enabled())).expect("hook");
    // UI levers default ON; the aggressive, non-UI levers stay off.
    assert!(hook.dynamic_thinking_enabled());
    assert!(!hook.dynamic_subagent_thinking_enabled());
    assert!(!hook.tournament_pruning_enabled());
    assert!(hook.distill_outputs_enabled());
    assert!(!hook.prune_dead_ends_enabled());
    assert!(hook.gate_tools_enabled());

    let ui_off = JudgmentConfig {
        dynamic_thinking: false,
        distill_outputs: false,
        gate_tools: false,
        ..enabled()
    };
    let hook = JudgmentHook::from_config(Some(&ui_off)).expect("hook");
    assert!(!hook.dynamic_thinking_enabled());
    assert!(!hook.distill_outputs_enabled());
    assert!(!hook.gate_tools_enabled());
    // The threshold is a value, not a lever: it is readable regardless.
    assert_eq!(hook.distill_line_threshold(), 40);

    let all_on = JudgmentConfig {
        dynamic_thinking: true,
        dynamic_subagent_thinking: true,
        tournament_pruning: true,
        distill_outputs: true,
        prune_dead_ends: true,
        gate_tools: true,
        distill_line_threshold: 12,
        ..enabled()
    };
    let hook = JudgmentHook::from_config(Some(&all_on)).expect("hook");
    assert!(hook.dynamic_thinking_enabled());
    assert!(hook.dynamic_subagent_thinking_enabled());
    assert!(hook.tournament_pruning_enabled());
    assert!(hook.distill_outputs_enabled());
    assert!(hook.prune_dead_ends_enabled());
    assert!(hook.gate_tools_enabled());
    assert_eq!(hook.distill_line_threshold(), 12);
}
