//! Credential-resolution tests for [`super::JevClient`].
//!
//! Every case mutates process env, so all are `#[serial]`.
//! Each test holds one guard per key it touches; [`EnvGuard`] restores its key on drop.

use super::*;
use serial_test::serial;
use xai_grok_test_support::EnvGuard;

/// Preferred fallback variable.
const TYPESAFE_ENV: &str = "TYPESAFE_API_KEY";
/// Secondary fallback variable.
const JEV_ENV: &str = "JEV_TYPESAFE_AI_KEY";

#[test]
#[serial]
fn explicit_literal_key_is_used_verbatim() {
    let _primary = EnvGuard::unset(TYPESAFE_ENV);
    let _secondary = EnvGuard::unset(JEV_ENV);
    assert_eq!(
        resolve_api_key(Some("sk-literal")).as_deref(),
        Some("sk-literal")
    );
}

#[test]
#[serial]
fn env_prefix_reference_reads_the_named_variable() {
    let _key = EnvGuard::set("JEV_TEST_KEY_VAR", "sk-from-env");
    let _primary = EnvGuard::unset(TYPESAFE_ENV);
    let _secondary = EnvGuard::unset(JEV_ENV);
    assert_eq!(
        resolve_api_key(Some("env:JEV_TEST_KEY_VAR")).as_deref(),
        Some("sk-from-env")
    );
}

/// A user who explicitly names a variable that is unset gets no judgment work, rather than a
/// silent fall through to an unrelated credential.
#[test]
#[serial]
fn env_prefix_reference_to_an_unset_variable_fails_closed() {
    let _missing = EnvGuard::unset("JEV_TEST_MISSING_VAR");
    let _primary = EnvGuard::set(TYPESAFE_ENV, "sk-unrelated");
    assert_eq!(resolve_api_key(Some("env:JEV_TEST_MISSING_VAR")), None);
}

#[test]
#[serial]
fn blank_configured_key_falls_back_to_the_environment() {
    let _primary = EnvGuard::set(TYPESAFE_ENV, "sk-typesafe");
    let _secondary = EnvGuard::unset(JEV_ENV);
    for blank in ["", "   "] {
        assert_eq!(
            resolve_api_key(Some(blank)).as_deref(),
            Some("sk-typesafe"),
            "configured value {blank:?} must be treated as unset"
        );
    }
}

#[test]
#[serial]
fn fallback_prefers_the_typesafe_variable() {
    let _primary = EnvGuard::set(TYPESAFE_ENV, "sk-typesafe");
    let _secondary = EnvGuard::set(JEV_ENV, "sk-jev");
    assert_eq!(resolve_api_key(None).as_deref(), Some("sk-typesafe"));
}

#[test]
#[serial]
fn fallback_uses_the_jev_variable_when_the_typesafe_one_is_absent() {
    let _primary = EnvGuard::unset(TYPESAFE_ENV);
    let _secondary = EnvGuard::set(JEV_ENV, "sk-jev");
    assert_eq!(resolve_api_key(None).as_deref(), Some("sk-jev"));
}

#[test]
#[serial]
fn no_configured_key_and_no_environment_leaves_judgment_disabled() {
    let _primary = EnvGuard::unset(TYPESAFE_ENV);
    let _secondary = EnvGuard::unset(JEV_ENV);
    assert_eq!(resolve_api_key(None), None);
    assert!(JevClient::try_new(None, "http://127.0.0.1:1".to_owned(), 400).is_none());
}

#[test]
#[serial]
fn try_new_builds_a_client_when_a_key_is_available() {
    let _primary = EnvGuard::unset(TYPESAFE_ENV);
    let _secondary = EnvGuard::unset(JEV_ENV);
    let client = JevClient::try_new(
        Some("sk-test".to_owned()),
        "http://127.0.0.1:1".to_owned(),
        400,
    )
    .expect("a configured key must yield a client");
    assert!(
        !format!("{client:?}").contains("sk-test"),
        "the credential must never reach a log line"
    );
}
