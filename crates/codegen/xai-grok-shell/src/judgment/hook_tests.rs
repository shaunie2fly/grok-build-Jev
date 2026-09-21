//! [`super::JudgmentHook`] gating tests.
//!
//! These construct configs directly, so no test mutates process env and none need `#[serial]`.
//! A credential is supplied literally; the endpoint is never contacted.

use super::*;

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
    // No `env:VAR` and no fallback variables set in this process ⇒ no credential ⇒ no hook.
    assert!(
        super::super::client::JevClient::try_new(None, ENDPOINT.to_owned(), 400).is_none(),
        "guard: this environment must expose no fallback credential for the assertion below"
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
