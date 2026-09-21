//! Seam tests for Subsystem 2 (`apply_dynamic_subagent_effort`).
//!
//! These hit the spawn helper directly with a scripted TypeSafe endpoint so a missing skip,
//! a raw effort assign, or classify-inside-model-resolution fails the assertion rather than
//! the network.

use super::super::*;
use crate::agent::config::{EndpointsConfig, ModelEntry};
use crate::judgment::JudgmentHook;
use crate::test_support::lsp_runtime::ctx_with_toggle;
use agent_client_protocol as acp;
use axum::{Json, Router, routing::post};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use xai_grok_config::JudgmentConfig;
use xai_grok_sampling_types::{ReasoningEffort, ReasoningEffortOption};

fn menu(efforts: &[ReasoningEffort]) -> Vec<ReasoningEffortOption> {
    efforts
        .iter()
        .map(|&value| ReasoningEffortOption {
            id: value.as_ref().to_string(),
            value,
            label: value.to_string(),
            description: None,
            default: false,
        })
        .collect()
}

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

async fn unreachable_endpoint() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}/")
}

fn score_answer(question: &str, score: f64) -> serde_json::Value {
    let mut answers = serde_json::Map::new();
    answers.insert(
        question.to_owned(),
        serde_json::json!({ "type": "score", "score": score }),
    );
    serde_json::json!({ "answers": answers })
}

fn session_id() -> acp::SessionId {
    acp::SessionId::new("child")
}

fn hook(endpoint: &str, dynamic_subagent_thinking: bool) -> JudgmentHook {
    JudgmentHook::from_config(Some(&JudgmentConfig {
        enabled: true,
        api_key: Some("sk-test".to_owned()),
        endpoint: endpoint.to_owned(),
        timeout_ms: 400,
        dynamic_subagent_thinking,
        ..JudgmentConfig::default()
    }))
    .expect("enabled + keyed ⇒ hook")
}

fn ctx_with_hook(hook: JudgmentHook, model: &str, supports_effort: bool) -> SubagentSpawnContext {
    let mut ctx = ctx_with_toggle(HashMap::new());
    ctx.judgment_hook = Some(Arc::new(hook));
    ctx.sampling_config.model = model.to_owned();
    ctx.sampling_config.reasoning_effort = Some(ReasoningEffort::Low);
    let mut entry = ModelEntry::fallback(model, &EndpointsConfig::default());
    entry.info.supports_reasoning_effort = supports_effort;
    ctx.models_manager.insert_test_entry(model, entry);
    ctx
}

fn inherited(effort: ReasoningEffort) -> xai_grok_sampler::SamplerConfig {
    xai_grok_sampler::SamplerConfig {
        model: "effort-model".to_owned(),
        reasoning_effort: Some(effort),
        context_window: 256_000,
        ..Default::default()
    }
}

async fn apply(
    config: &mut xai_grok_sampler::SamplerConfig,
    subagent_type: &str,
    prompt: &str,
    ctx: &SubagentSpawnContext,
    explicit: Option<&str>,
) {
    apply_dynamic_subagent_effort(config, subagent_type, prompt, ctx, explicit, &session_id())
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lever_off_leaves_inherited_effort() {
    let endpoint = unreachable_endpoint().await;
    let ctx = ctx_with_hook(hook(&endpoint, false), "effort-model", true);
    let mut config = inherited(ReasoningEffort::High);
    apply(
        &mut config,
        "general-purpose",
        "refactor the scheduler",
        &ctx,
        None,
    )
    .await;
    assert_eq!(config.reasoning_effort, Some(ReasoningEffort::High));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_prompt_leaves_inherited_effort() {
    let endpoint = unreachable_endpoint().await;
    let ctx = ctx_with_hook(hook(&endpoint, true), "effort-model", true);
    let mut config = inherited(ReasoningEffort::High);
    apply(&mut config, "explore", "   \n", &ctx, None).await;
    assert_eq!(
        config.reasoning_effort,
        Some(ReasoningEffort::High),
        "no task text must not pin explore to low"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explore_pins_low_without_calling_jev() {
    let endpoint = unreachable_endpoint().await;
    let ctx = ctx_with_hook(hook(&endpoint, true), "effort-model", true);
    let mut config = inherited(ReasoningEffort::High);
    apply(&mut config, "explore", "map the architecture", &ctx, None).await;
    assert_eq!(config.reasoning_effort, Some(ReasoningEffort::Low));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mapped_score_is_applied_through_supported_effort() {
    let (endpoint, seen) = serve(score_answer("complexity", 3.0)).await;
    let ctx = ctx_with_hook(hook(&endpoint, true), "effort-model", true);
    let mut config = inherited(ReasoningEffort::Low);
    apply(
        &mut config,
        "general-purpose",
        "refactor the scheduler",
        &ctx,
        None,
    )
    .await;
    assert_eq!(config.reasoning_effort, Some(ReasoningEffort::Xhigh));
    assert_eq!(seen.lock().len(), 1, "one classify call");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mapped_score_stays_inside_a_three_level_menu() {
    let (endpoint, _seen) = serve(score_answer("complexity", 3.0)).await;
    let mut ctx = ctx_with_hook(hook(&endpoint, true), "grok-4.5", true);
    ctx.models_manager.insert_test_entry("grok-4.5", {
        let mut entry = ModelEntry::fallback("grok-4.5", &EndpointsConfig::default());
        entry.info.supports_reasoning_effort = true;
        entry.info.reasoning_efforts = menu(&[
            ReasoningEffort::High,
            ReasoningEffort::Medium,
            ReasoningEffort::Low,
        ]);
        entry
    });
    let mut config = inherited(ReasoningEffort::Low);
    config.model = "grok-4.5".to_owned();
    apply(
        &mut config,
        "general-purpose",
        "refactor the scheduler",
        &ctx,
        None,
    )
    .await;
    assert_eq!(
        config.reasoning_effort,
        Some(ReasoningEffort::High),
        "a model without xhigh must not be stamped xhigh"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn classify_none_keeps_inherited_effort() {
    let endpoint = unreachable_endpoint().await;
    let ctx = ctx_with_hook(hook(&endpoint, true), "effort-model", true);
    let mut config = inherited(ReasoningEffort::High);
    apply(
        &mut config,
        "general-purpose",
        "refactor the scheduler",
        &ctx,
        None,
    )
    .await;
    assert_eq!(config.reasoning_effort, Some(ReasoningEffort::High));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_effort_skips_jev() {
    let (endpoint, seen) = serve(score_answer("complexity", 3.0)).await;
    let ctx = ctx_with_hook(hook(&endpoint, true), "effort-model", true);
    let mut config = inherited(ReasoningEffort::Medium);
    apply(
        &mut config,
        "explore",
        "map the architecture",
        &ctx,
        Some("high"),
    )
    .await;
    assert_eq!(
        config.reasoning_effort,
        Some(ReasoningEffort::Medium),
        "explicit skip must not classify or overwrite; the caller already applied the override"
    );
    assert!(
        seen.lock().is_empty(),
        "explicit spawn effort must not spend a TypeSafe call"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_model_ignores_jev_verdict() {
    let endpoint = unreachable_endpoint().await;
    let ctx = ctx_with_hook(hook(&endpoint, true), "plain-model", false);
    let mut config = inherited(ReasoningEffort::High);
    config.model = "plain-model".to_owned();
    apply(&mut config, "explore", "map the architecture", &ctx, None).await;
    assert_eq!(
        config.reasoning_effort,
        Some(ReasoningEffort::High),
        "apply_supported_effort must drop a Jev verdict the catalog does not support"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_resolution_does_not_classify() {
    use xai_grok_agent::config::ModelOverride;
    let endpoint = unreachable_endpoint().await;
    let mut ctx = ctx_with_hook(hook(&endpoint, true), "effort-model", true);
    ctx.sampling_config.reasoning_effort = Some(ReasoningEffort::High);
    let (config, _) =
        resolve_effective_model_config(None, "explore", &ModelOverride::Inherit, &ctx).await;
    assert_eq!(
        config.reasoning_effort,
        Some(ReasoningEffort::High),
        "classify runs after model fallback/resume/explicit, not inside model resolution"
    );
}
