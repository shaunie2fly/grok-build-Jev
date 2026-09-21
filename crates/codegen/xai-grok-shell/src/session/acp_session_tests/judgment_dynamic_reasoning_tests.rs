//! Subsystem 1 is parent-only. A child session must keep spawn-time effort even when
//! `dynamic_thinking` is on and Jev would return a different classification.

use super::support::*;
use super::*;
use crate::agent::config::{EndpointsConfig, ModelEntry};
use crate::judgment::JudgmentHook;
use axum::{Json, Router, routing::post};
use parking_lot::Mutex;
use std::sync::Arc;
use xai_grok_config::JudgmentConfig;
use xai_grok_sampling_types::{ConversationItem, ReasoningEffort};

fn seed_thinking_model(actor: &SessionActor, id: &str) {
    let mut entry = ModelEntry::fallback(id, &EndpointsConfig::default());
    entry.info.supports_reasoning_effort = true;
    actor.models_manager.insert_test_entry(id, entry);
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

fn score_answer(score: f64) -> serde_json::Value {
    serde_json::json!({
        "answers": {
            "complexity": { "type": "score", "score": score }
        }
    })
}

fn hook(endpoint: &str) -> JudgmentHook {
    JudgmentHook::from_config(Some(&JudgmentConfig {
        enabled: true,
        api_key: Some("sk-test".to_owned()),
        endpoint: endpoint.to_owned(),
        timeout_ms: 400,
        ..JudgmentConfig::default()
    }))
    .expect("enabled + keyed ⇒ hook")
}

#[tokio::test(flavor = "current_thread")]
async fn subagent_session_does_not_reclassify_spawn_effort() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (endpoint, seen) = serve(score_answer(3.0)).await;
            let mut actor = plain_actor().await;
            actor
                .chat_state_handle
                .push_user_message(ConversationItem::user("map the architecture of this repo"));
            actor.startup_hints.is_subagent = true;
            actor.startup_hints.subagent_type = Some("explore".to_owned());
            assert!(
                actor.judgment_hook.set(Some(hook(&endpoint))).is_ok(),
                "hook set once"
            );

            let mut config = xai_grok_sampler::SamplerConfig {
                reasoning_effort: Some(ReasoningEffort::Low),
                context_window: 256_000,
                ..Default::default()
            };
            actor.apply_dynamic_reasoning_effort(&mut config).await;
            assert_eq!(
                config.reasoning_effort,
                Some(ReasoningEffort::Low),
                "child turns must keep spawn-time effort; Subsystem 1 is parent-only"
            );
            assert!(
                seen.lock().is_empty(),
                "a subagent session must not spend a TypeSafe call on dynamic_thinking"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn parent_session_still_classifies_when_dynamic_thinking_is_on() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (endpoint, seen) = serve(score_answer(3.0)).await;
            let actor = plain_actor().await;
            seed_thinking_model(&actor, "effort-model");
            actor
                .chat_state_handle
                .push_user_message(ConversationItem::user("refactor the scheduler"));
            assert!(
                actor.judgment_hook.set(Some(hook(&endpoint))).is_ok(),
                "hook set once"
            );

            let mut config = xai_grok_sampler::SamplerConfig {
                model: "effort-model".to_owned(),
                reasoning_effort: Some(ReasoningEffort::Low),
                context_window: 256_000,
                ..Default::default()
            };
            actor.apply_dynamic_reasoning_effort(&mut config).await;
            assert_eq!(config.reasoning_effort, Some(ReasoningEffort::Xhigh));
            assert_eq!(seen.lock().len(), 1);
        })
        .await;
}
