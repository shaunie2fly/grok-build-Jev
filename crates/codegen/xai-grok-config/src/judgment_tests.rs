use super::JudgmentConfig;

/// Spec §3.3 sample configuration.
const SAMPLE: &str = r#"
enabled = true
api_key = "env:TYPESAFE_API_KEY"

dynamic_thinking = true
dynamic_subagent_thinking = true
tournament_pruning = true
distill_outputs = true
distill_line_threshold = 40
prune_dead_ends = true

gate_tools = true
safety_threshold = 0.20
"#;

#[test]
fn sample_config_enables_every_lever() {
    let config: JudgmentConfig = toml::from_str(SAMPLE).unwrap();
    assert!(config.enabled);
    assert_eq!(config.api_key.as_deref(), Some("env:TYPESAFE_API_KEY"));
    assert!(config.dynamic_thinking);
    assert!(config.dynamic_subagent_thinking);
    assert!(config.tournament_pruning);
    assert!(config.distill_outputs);
    assert!(config.prune_dead_ends);
    assert!(config.gate_tools);
}

/// A user who writes only `enabled = true` must still get the documented endpoint, timeout, and
/// thresholds; the per-field serde defaults are what keeps a partial table usable.
#[test]
fn partial_table_fills_every_unset_field_from_the_documented_default() {
    let partial: JudgmentConfig = toml::from_str("enabled = true\n").unwrap();
    assert!(partial.enabled);
    assert_eq!(partial.endpoint, "https://api.typesafe.ai/v1/systemone");
    assert_eq!(partial.timeout_ms, 1500);
    assert_eq!(partial.distill_line_threshold, 40);
    assert_eq!(partial.safety_threshold, 0.08);
    assert_eq!(partial.api_key, None);
    // UI-exposed levers default ON so `[judgment] enabled = true` alone matches the Settings
    // modal: toggling the master switch produces the documented feature set.
    assert!(
        partial.dynamic_thinking && partial.distill_outputs && partial.gate_tools,
        "unset UI levers must default on, got thinking={} distill={} gate={}",
        partial.dynamic_thinking,
        partial.distill_outputs,
        partial.gate_tools
    );
    // Aggressive, non-UI levers stay off until the user opts in through TOML.
    assert!(!partial.dynamic_subagent_thinking);
    assert!(!partial.tournament_pruning);
    assert!(!partial.prune_dead_ends);

    // A programmatically-built default must land on the same values, or the two construction
    // paths drift (and an empty endpoint / zero timeout fails every call).
    let built = JudgmentConfig {
        enabled: true,
        ..JudgmentConfig::default()
    };
    assert_eq!(built, partial);

    // Absent section: deserializing an empty table is the "section omitted" case.
    assert_eq!(
        toml::from_str::<JudgmentConfig>("").unwrap(),
        JudgmentConfig::default()
    );
    assert!(!JudgmentConfig::default().enabled);
}

#[test]
fn default_serializes_to_a_table_that_reparses_to_itself() {
    // The shell loader round-trips `Config::default()` through `toml::Value::try_from`, so a
    // `None` field must not poison serialization.
    let value = toml::Value::try_from(JudgmentConfig::default()).unwrap();
    let table = value.as_table().unwrap();
    assert!(
        !table.contains_key("api_key"),
        "an unset key must be omitted, not serialized as a null"
    );
    assert_eq!(
        toml::from_str::<JudgmentConfig>(&toml::to_string(&JudgmentConfig::default()).unwrap())
            .unwrap(),
        JudgmentConfig::default()
    );
}

#[test]
fn configured_values_round_trip_through_toml() {
    let config: JudgmentConfig = toml::from_str(SAMPLE).unwrap();
    let serialized = toml::to_string(&config).unwrap();
    assert_eq!(
        toml::from_str::<JudgmentConfig>(&serialized).unwrap(),
        config
    );
}

/// The default request budget must clear the endpoint's real latency, not just its happy path.
///
/// Measured against the live Jev endpoint: p50 ~465 ms, p95 ~568 ms, p99 ~631 ms warm, with cold
/// calls near 1200 ms. A 400 ms budget expired on 20/20 observed calls, which disabled every
/// subsystem while still looking configured — a silent failure with no signal to the user. This
/// test pins the default above the measured p99 so that regression cannot recur unnoticed.
#[test]
fn the_default_timeout_clears_the_measured_endpoint_latency() {
    let default = JudgmentConfig::default().timeout_ms;
    const MEASURED_P99_MS: u64 = 631;
    assert!(
        default >= MEASURED_P99_MS,
        "default {default} ms is below the measured p99 ({MEASURED_P99_MS} ms): most calls would time out"
    );
    // It should also round-trip through serde, since a partial `[judgment]` table is the common case.
    let partial: JudgmentConfig = toml::from_str("enabled = true\n").unwrap();
    assert_eq!(partial.timeout_ms, default);
}
