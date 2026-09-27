//! [`super::JudgmentEvaluator`] tests.
//!
//! Two halves:
//! - **Graceful degradation** — an unreachable, hung, erroring, or malformed endpoint must yield
//!   each caller's pre-Jev default, never an error and never a stall past the configured timeout.
//! - **Thresholds** — the score-to-decision mapping for each subsystem, including its boundaries.
//!
//! These construct a client with an explicit literal key, so none of them touch process env and
//! none need `#[serial]`.

use super::*;
use crate::judgment::client::JevClient;
use axum::{Json, Router, http::StatusCode, routing::post};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use xai_grok_sampling_types::ReasoningEffort;

/// Serve every POST with a scripted body, capturing the request JSON.
async fn serve(response: serde_json::Value) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let capture = seen.clone();
    let app = Router::new().route(
        "/",
        post(move |Json(body): Json<serde_json::Value>| {
            let capture = capture.clone();
            let response = response.clone();
            async move {
                capture.lock().push(body);
                Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/"), seen)
}

/// Serve every POST with a fixed status and body.
async fn serve_status(status: StatusCode, body: &'static str) -> String {
    let app = Router::new().route("/", post(move || async move { (status, body.to_owned()) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/")
}

/// A port nothing is listening on: bind, learn the address, release it.
async fn unreachable_endpoint() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}/")
}

/// Accepts connections but never answers, standing in for a hung or black-holed endpoint.
async fn blackhole_endpoint() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                drop(stream);
            });
        }
    });
    format!("http://{addr}/")
}

fn evaluator(endpoint: String, timeout_ms: u64) -> JudgmentEvaluator {
    let client = JevClient::try_new(Some("sk-test".to_owned()), endpoint, timeout_ms)
        .expect("an explicit literal key must build a client");
    JudgmentEvaluator::new(client, 0.20)
}

/// Grok 4.6 thinking menu (and the catalog fallback when `reasoning_efforts` is empty).
fn grok46() -> [ReasoningEffort; 4] {
    [
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
        ReasoningEffort::Xhigh,
    ]
}

/// Grok 4.5 thinking menu: no `xhigh`.
fn grok45() -> [ReasoningEffort; 3] {
    [
        ReasoningEffort::High,
        ReasoningEffort::Medium,
        ReasoningEffort::Low,
    ]
}

/// A `score` answer keyed by question id.
/// Built through an explicit map: the `json!` macro treats a bare identifier key as a literal
/// string, so a dynamic key must not be written inside the macro.
fn score_answer(question: &str, score: f64) -> serde_json::Value {
    answer(
        question,
        serde_json::json!({ "type": "score", "score": score }),
    )
}

/// A `noul` answer keyed by question id.
fn noul_answer(question: &str, noul: f64) -> serde_json::Value {
    answer(
        question,
        serde_json::json!({ "type": "noul", "noul": noul }),
    )
}

fn answer(question: &str, body: serde_json::Value) -> serde_json::Value {
    let mut answers = serde_json::Map::new();
    answers.insert(question.to_owned(), body);
    serde_json::json!({ "answers": answers })
}

/// The whole point of the integration: if Jev is unreachable, every subsystem keeps vanilla
/// behavior instead of erroring or blocking the turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_subsystem_falls_back_when_the_endpoint_is_unreachable() {
    let eval = evaluator(unreachable_endpoint().await, 400);

    assert_eq!(
        eval.classify_reasoning("do a thing", None, &grok46()).await,
        None,
        "an outage must not substitute a fabricated effort"
    );
    assert!(
        eval.has_actionable_errors("error: boom").await,
        "an unknown output must be kept, not distilled"
    );
    assert_eq!(eval.score_patch("task", "diff").await, 0.5);
    assert!(!eval.verify_dead_end("summary").await);
    assert!(
        !eval.is_safe_tool("bash", "rm -rf /").await,
        "safety gating must fail closed"
    );
}

/// A hung endpoint must be cut off by the configured timeout rather than stalling the turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hung_endpoint_is_bounded_by_the_configured_timeout() {
    let endpoint = blackhole_endpoint().await;
    let eval = evaluator(endpoint, 150);

    let started = Instant::now();
    assert_eq!(
        eval.classify_reasoning("do a thing", None, &grok46()).await,
        None,
        "a hung endpoint must not substitute a fabricated effort"
    );
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(2),
        "a 150 ms budget must not block the turn, took {elapsed:?}"
    );
}

/// An HTTP error must take the fallback path. Without an explicit status check a JSON error body
/// would parse as a response with no score and silently mask a rejected key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_errors_and_malformed_bodies_take_the_fallback_path() {
    // 401 with a JSON body shaped like a valid-looking response.
    let unauthorized = serve_status(
        StatusCode::UNAUTHORIZED,
        r#"{"answers":{"complexity":{"type":"score","score":3.0}}}"#,
    )
    .await;
    let eval = evaluator(unauthorized, 400);
    assert_eq!(
        eval.classify_reasoning("do a thing", None, &grok46()).await,
        None,
        "a rejected key must not read as a top-difficulty score"
    );

    let server_error = serve_status(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let eval = evaluator(server_error, 400);
    assert_eq!(eval.score_patch("task", "diff").await, 0.5);

    // A 200 whose body is not JSON at all.
    let not_json = serve_status(StatusCode::OK, "definitely not json").await;
    let eval = evaluator(not_json, 400);
    assert_eq!(
        eval.classify_reasoning("do a thing", None, &grok46()).await,
        None
    );
    assert!(eval.has_actionable_errors("error: boom").await);
    assert!(!eval.verify_dead_end("summary").await);
    assert!(!eval.is_safe_tool("bash", "ls").await);
}

/// A 200 with a well-formed but answer-less body must not be mistaken for a strong verdict.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_response_missing_the_answer_uses_the_conservative_default() {
    let (endpoint, _seen) = serve(serde_json::json!({ "answers": {} })).await;
    let eval = evaluator(endpoint, 400);

    assert_eq!(
        eval.classify_reasoning("do a thing", None, &grok46()).await,
        None
    );
    assert!(eval.has_actionable_errors("error: boom").await);
    assert_eq!(eval.score_patch("task", "diff").await, 0.0);
    assert!(!eval.verify_dead_end("summary").await);
    assert!(!eval.is_safe_tool("bash", "ls").await);
}

/// An `explore` subagent only reads and reports, so its effort is pinned to the cheapest
/// Grok picker level without a network call.
/// Pointing at an unreachable endpoint proves no call is made: a call would return `None`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explore_subagents_pin_low_without_calling_jev() {
    let eval = evaluator(unreachable_endpoint().await, 400);
    assert_eq!(
        eval.classify_reasoning("anything at all", Some("explore"), &grok46())
            .await,
        Some("low")
    );
}

/// The request must match the real System One contract: `instructions` (not `prompt`), and a
/// `criteria` array for a `score` question.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_follow_the_system_one_wire_contract() {
    let (endpoint, seen) = serve(score_answer("complexity", 0.0)).await;
    let eval = evaluator(endpoint, 400);
    eval.classify_reasoning("refactor the scheduler", None, &grok46())
        .await;

    let requests = seen.lock();
    let request = requests.first().expect("one request must be sent");
    assert_eq!(request["model"], "jev-latest");
    let question = &request["questions"]["complexity"];
    assert_eq!(question["type"], "score");
    assert!(
        question["instructions"].is_string(),
        "system-one takes `instructions`, got {question}"
    );
    assert!(
        question["criteria"]
            .as_array()
            .is_some_and(|c| c.len() == 4),
        "a score question needs ordered criteria, got {question}"
    );
    // The prompt reaches Jev as state, so it is still part of the judgment context.
    assert_eq!(request["state"]["prompt"], "refactor the scheduler");
}

/// Reasoning thresholds: raw level-weighted scores normalized onto 0.0-1.0, then onto equal-width
/// bands of the model's thinking menu (cheapest first). For the Grok 4.6 four-level menu the
/// edges are 0.25 / 0.50 / 0.75. `"minimal"` is not on that menu and must not appear.
/// Raw values are chosen so `raw / 3.0` lands on an exactly representable fraction.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn classify_reasoning_maps_scores_onto_effort_levels() {
    for (raw, expected) in [
        (0.0, "low"),
        (0.5, "low"),
        (0.75, "medium"), // exactly 0.25: the low band is exclusive at its upper edge
        (1.5, "high"),    // exactly 0.50
        (2.25, "xhigh"),  // exactly 0.75
        (3.0, "xhigh"),   // exactly 1.0
    ] {
        let (endpoint, _seen) = serve(score_answer("complexity", raw)).await;
        let eval = evaluator(endpoint, 400);
        assert_eq!(
            eval.classify_reasoning("prompt", None, &grok46()).await,
            Some(expected),
            "raw score {raw} should map to {expected}"
        );
    }
}

#[test]
fn score_bands_follow_the_active_models_menu() {
    assert_eq!(map_score_onto_offered_efforts(0.0, &[]), None);
    assert_eq!(
        map_score_onto_offered_efforts(1.0, &grok46()),
        Some("xhigh")
    );
    assert_eq!(map_score_onto_offered_efforts(1.0, &grok45()), Some("high"));
    assert_eq!(map_score_onto_offered_efforts(0.0, &grok45()), Some("low"));
    assert_eq!(
        map_score_onto_offered_efforts(1.0, &[ReasoningEffort::High]),
        Some("high")
    );
}

#[test]
fn explore_pin_is_the_cheapest_row_on_the_models_menu() {
    assert_eq!(cheapest_offered_effort(&[]), None);
    assert_eq!(cheapest_offered_effort(&grok45()), Some("low"));
    assert_eq!(
        cheapest_offered_effort(&[ReasoningEffort::High]),
        Some("high")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_menu_does_not_call_jev() {
    let eval = evaluator(unreachable_endpoint().await, 400);
    assert_eq!(eval.classify_reasoning("prompt", None, &[]).await, None);
    assert_eq!(
        eval.classify_reasoning("prompt", Some("explore"), &[])
            .await,
        None
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explore_pins_the_only_offered_level_when_the_menu_is_a_singleton() {
    let eval = evaluator(unreachable_endpoint().await, 400);
    assert_eq!(
        eval.classify_reasoning("anything", Some("explore"), &[ReasoningEffort::High])
            .await,
        Some("high")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actionable_errors_split_on_a_coin_flip() {
    for (noul, expected) in [(0.9, true), (0.51, true), (0.5, false), (0.0, false)] {
        let (endpoint, _seen) = serve(noul_answer("has_errors", noul)).await;
        let eval = evaluator(endpoint, 400);
        assert_eq!(
            eval.has_actionable_errors("output").await,
            expected,
            "noul {noul} should map to {expected}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patch_quality_scores_normalize_onto_zero_to_one() {
    for (raw, expected) in [(0.0, 0.0), (1.5, 0.5), (3.0, 1.0)] {
        let (endpoint, _seen) = serve(score_answer("quality", raw)).await;
        let eval = evaluator(endpoint, 400);
        let score = eval.score_patch("task", "diff").await;
        assert!(
            (score - expected).abs() < 1e-9,
            "raw {raw} should normalize to {expected}, got {score}"
        );
    }
}

/// Pruning is destructive, so it needs a confident verdict: strictly above 0.75.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dead_end_verification_requires_a_confident_verdict() {
    for (noul, expected) in [(0.9, true), (0.76, true), (0.75, false), (0.4, false)] {
        let (endpoint, _seen) = serve(noul_answer("is_abandoned", noul)).await;
        let eval = evaluator(endpoint, 400);
        assert_eq!(
            eval.verify_dead_end("summary").await,
            expected,
            "noul {noul} should map to {expected}"
        );
    }
}

/// Safety gating compares the destructiveness probability against the threshold on the evaluator.
/// The helper pins that threshold at `0.20` (an `f32`, so it widens to 0.20000000298023224); the
/// cases stay clear of that edge. The schema default is `0.08` and is pinned separately. The
/// exclusive comparison itself is covered by the representable-threshold test below.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_safety_gating_honors_the_configured_threshold() {
    for (noul, expected) in [(0.0, true), (0.19, true), (0.21, false), (0.9, false)] {
        let (endpoint, _seen) = serve(noul_answer("is_destructive", noul)).await;
        let eval = evaluator(endpoint, 400);
        assert_eq!(
            eval.is_safe_tool("bash", "ls").await,
            expected,
            "risk {noul} against a 0.20 threshold should map to {expected}"
        );
    }
}

/// The schema default threshold must stay below the score of genuinely destructive commands.
///
/// Measured on the live Jev endpoint (n=12 per command): benign commands score 0.02–0.07 while
/// destructive ones score 0.10–0.59, with no overlap. The default is what a Settings-modal
/// opt-in installs, so it must not sit inside the destructive band — the previous 0.20 let
/// `dd if=/dev/zero of=…` (0.10) and `chown -R root:root …` (0.16) through without asking.
#[test]
fn the_default_threshold_sits_below_the_measured_destructive_band() {
    let default = xai_grok_config::JudgmentConfig::default().safety_threshold;
    assert!(
        default > 0.07,
        "default {default} would ask the user to re-approve ordinary benign commands"
    );
    assert!(
        default <= 0.10,
        "default {default} would auto-approve commands measured as destructive (lowest observed 0.10)"
    );
}

/// The threshold comparison is exclusive: a risk exactly equal to an exactly representable
/// threshold counts as unsafe, so a call can never be auto-approved at the configured limit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_risk_exactly_at_a_representable_threshold_is_not_safe() {
    let (endpoint, _seen) = serve(noul_answer("is_destructive", 0.5)).await;
    let client = JevClient::try_new(Some("sk-test".to_owned()), endpoint, 400).unwrap();
    let evaluator = JudgmentEvaluator::new(client, 0.5);
    assert!(
        !evaluator.is_safe_tool("bash", "ls").await,
        "risk == threshold must not auto-approve"
    );
}

/// A caller-supplied threshold, not just the `safety_threshold` default, must be what gates.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_custom_safety_threshold_changes_the_verdict() {
    let (endpoint, _seen) = serve(noul_answer("is_destructive", 0.5)).await;
    let client = JevClient::try_new(Some("sk-test".to_owned()), endpoint, 400).unwrap();
    assert!(
        JudgmentEvaluator::new(client, 0.6)
            .is_safe_tool("bash", "ls")
            .await
    );
}

#[test]
fn truncation_counts_characters_not_bytes() {
    // 10 multi-byte characters truncated to 4 must yield 4 characters, never a split codepoint.
    assert_eq!(truncate("éééééééééé", 4), "éééé");
    assert_eq!(truncate("abc", 5), "abc");
}
