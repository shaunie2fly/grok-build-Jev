//! TypeSafe Jev HTTP client: a thin wrapper over the system-one judgment endpoint.
//!
//! Every call is bounded by the configured timeout, so a slow or unreachable endpoint falls back
//! to default behavior instead of stalling a turn.

use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;
use xai_grok_env::env_string;

/// Environment variables consulted, in order, when `[judgment] api_key` is unset or blank.
const FALLBACK_API_KEY_ENV_VARS: &[&str] = &["TYPESAFE_API_KEY", "JEV_TYPESAFE_AI_KEY"];

/// Prefix marking an `api_key` value as an environment-variable reference (`env:VAR_NAME`).
const ENV_KEY_PREFIX: &str = "env:";

/// The system-one model id every call sends.
const JUDGMENT_MODEL: &str = "jev-latest";

/// A failed Jev call. Callers treat any error as "no judgment available" and apply their fallback.
#[derive(Debug, thiserror::Error)]
pub enum JevError {
    /// Transport, timeout, or response-decode failure.
    #[error("Jev request failed: {0}")]
    Transport(#[from] reqwest::Error),
    /// The endpoint answered, but not with a success status.
    #[error("Jev endpoint returned HTTP {0}")]
    Http(reqwest::StatusCode),
}

/// One Jev client, bound to a credential and endpoint resolved at construction.
/// [`Self::try_new`] returns `None` when no credential is available, which is how an unconfigured
/// install stays on vanilla behavior without any call site needing to check the environment.
#[derive(Clone)]
pub struct JevClient {
    client: Client,
    endpoint: String,
    api_key: String,
}

// Manual impl: the derived one would print the bearer token, and this struct is reachable from
// session state that gets logged on failure.
impl std::fmt::Debug for JevClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevClient")
            .field("endpoint", &self.endpoint)
            .field("api_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl JevClient {
    /// Build a client, resolving the credential in this order:
    /// an explicit `env:VAR_NAME` reference, an explicit literal key, then
    /// [`FALLBACK_API_KEY_ENV_VARS`].
    /// `None` means "no credential", so the caller does no judgment work at all.
    pub fn try_new(api_key: Option<String>, endpoint: String, timeout_ms: u64) -> Option<Self> {
        let api_key = resolve_api_key(api_key.as_deref())?;
        // Built through the shared helper so the judgment endpoint gets the same TLS policy
        // (extra roots, rustls provider) as every other client in this crate.
        let client = xai_grok_extra_ca::build_reqwest_client(|builder| {
            builder.timeout(Duration::from_millis(timeout_ms))
        })
        .ok()?;
        Some(Self {
            client,
            endpoint,
            api_key,
        })
    }

    /// POST one state/questions pair and return the decoded response.
    /// A non-success status is an error rather than a parse of the error body, so a rejected key
    /// surfaces as a warning instead of silently reading as a missing score.
    pub async fn evaluate(&self, state: Value, questions: Value) -> Result<Value, JevError> {
        let payload = json!({
            "model": JUDGMENT_MODEL,
            "state": state,
            "questions": questions
        });

        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(JevError::Http(status));
        }

        Ok(response.json::<Value>().await?)
    }
}

/// Resolve the credential.
/// An explicitly configured `env:VAR_NAME` that resolves to nothing fails closed: the user named a
/// credential, so a silent fall through to an unrelated variable would be a surprise.
fn resolve_api_key(configured: Option<&str>) -> Option<String> {
    match configured.map(str::trim) {
        Some(value) if !value.is_empty() => match value.strip_prefix(ENV_KEY_PREFIX) {
            Some(var_name) => env_string(var_name.trim()),
            None => Some(value.to_string()),
        },
        _ => FALLBACK_API_KEY_ENV_VARS
            .iter()
            .find_map(|name| env_string(name)),
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
