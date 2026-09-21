# Technical Specification: Jev Token Optimization & Execution Gating in Grok Build

## 1. Executive Summary & Core Mandate

The objective of this integration is to significantly reduce token consumption and operational latency in `grok-build` by delegating micro-decisions, classifications, and candidate evaluations to **TypeSafe Jev** (a sub-300ms, non-generative System 1 judgment model).

### The "Zero-Regression" Guarantee
1. **Disabled by Default:** Every Jev feature is gated behind `[judgment]` configuration flags. If the section is omitted, `enabled = false`, or `TYPESAFE_API_KEY` is absent, `grok-build` **MUST** behave identically to vanilla upstream.
2. **Fail-Open & Resilient Fallback:** Every network call to Jev is wrapped in a strict timeout ($\le 400\text{ms}$). If Jev times out, returns an HTTP error, or yields malformed JSON, the runtime **MUST** log a trace warning and immediately fall back to default `grok-build` behavior (e.g., retain full context, inherit parent reasoning effort, or prompt the user).
3. **Workspace Integrity:** The root `Cargo.toml` is generated and read-only. **DO NOT** edit the root `Cargo.toml`. Add dependencies strictly inside specific crate manifests (`xai-grok-config`, `xai-grok-shell`, `xai-grok-tools`).
4. **Prompt Cache Preservation:** Context pruning is restricted strictly to **contiguous trailing turns** resulting in net file reversions. Historical prefix turn sequences must remain unmodified to preserve frontier model prompt-cache hit rates.

---

## 2. Architecture & Call Topology

```
                                  ~/.grok/config.toml
                                           │
                                           ▼
                       crates/codegen/xai-grok-config/src/judgment.rs
                                           │
                                           ▼
                       agent/judgment_config.rs :: resolve_judgment_hook()
                       (overlay-free disk load; GROK_CONFIG cannot opt in)
                                           │
                 ┌─────────────────────────┴─────────────────────────┐
                 ▼                                                   ▼
     session OnceLock (parent)                    subagent spawn context
     sampler_turn / tool_calls                    ChildRunner seam
                 │                                                   │
                 └───────────────────────┬───────────────────────────┘
                                         ▼
                    xai-grok-shell::judgment::{JevClient, JudgmentEvaluator}
                                         ▼
                          https://api.typesafe.ai/v1/systemone
```

`xai-grok-tools` is **not** modified. Subagent reasoning effort is applied in the parent spawn path. Tournament ranking is a tested primitive (`judgment/tournament.rs`) with no executor in this repo.

Settings UI (pager) mirrors four keys under `SettingCategory::Agent`. TOML names and UI names differ:

| UI key | TOML field |
|---|---|
| `judgment.enabled` | `enabled` |
| `judgment.safety_gate_enabled` | `gate_tools` |
| `judgment.dynamic_reasoning_enabled` | `dynamic_thinking` |
| `judgment.distillation_enabled` | `distill_outputs` |

`dynamic_subagent_thinking`, `tournament_pruning`, and `prune_dead_ends` are TOML-only.

---

## 3. Configuration Specification

### Crate: `crates/codegen/xai-grok-config`

#### 3.1 Crate Manifest Updates
Ensure `serde` and `serde_json` are present in `crates/codegen/xai-grok-config/Cargo.toml`.

#### 3.2 Schema Definition
Create `crates/codegen/xai-grok-config/src/judgment.rs`:

Hand-write `Default` so it matches serde defaults. `#[derive(Default)]` would yield `endpoint = ""` and `timeout_ms = 0`.

UI-exposed levers default **true** (`dynamic_thinking`, `distill_outputs`, `gate_tools`) so `[judgment] enabled = true` or a Settings master-switch toggle produces the documented feature set. Aggressive levers (`dynamic_subagent_thinking`, `tournament_pruning`, `prune_dead_ends`) default **false**. `enabled` defaults **false**. `api_key` uses `skip_serializing_if = "Option::is_none"`.

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct JudgmentConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_true")]
    pub dynamic_thinking: bool,
    #[serde(default)]
    pub dynamic_subagent_thinking: bool,
    #[serde(default)]
    pub tournament_pruning: bool,
    #[serde(default = "default_true")]
    pub distill_outputs: bool,
    #[serde(default = "default_distill_threshold")]
    pub distill_line_threshold: usize,
    #[serde(default)]
    pub prune_dead_ends: bool,
    #[serde(default = "default_true")]
    pub gate_tools: bool,
    #[serde(default = "default_safety_threshold")]
    pub safety_threshold: f32,
}
```

Re-export `JudgmentConfig` in `crates/codegen/xai-grok-config/src/lib.rs` and attach `pub judgment: Option<JudgmentConfig>` to the top-level configuration struct.

#### 3.3 Sample Configuration (`~/.grok/config.toml`)

```toml
[judgment]
enabled = true
api_key = "env:TYPESAFE_API_KEY"

# Token Optimization Levers
dynamic_thinking = true
dynamic_subagent_thinking = true
tournament_pruning = true
distill_outputs = true
distill_line_threshold = 40
prune_dead_ends = true

# Execution Safety Levers
gate_tools = true
safety_threshold = 0.20
```

---

## 4. Jev Client & Evaluator Engine

### Crate: `crates/codegen/xai-grok-shell`

#### 4.1 Crate Manifest Updates
Add to `crates/codegen/xai-grok-shell/Cargo.toml`:
```toml
[dependencies]
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde_json = "1.0"
```

#### 4.2 HTTP Client Wrapper
Create `crates/codegen/xai-grok-shell/src/judgment/client.rs`:

```rust
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct JevClient {
    client: Client,
    endpoint: String,
    api_key: String,
}

impl JevClient {
    pub fn try_new(api_key: Option<String>, endpoint: String, timeout_ms: u64) -> Option<Self> {
        let key = match api_key {
            Some(ref k) if k.starts_with("env:") => {
                let var_name = &k[4..];
                std::env::var(var_name).ok()?
            }
            Some(k) if !k.trim().is_empty() => k,
            _ => std::env::var("TYPESAFE_API_KEY").ok()?,
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .ok()?;

        Some(Self {
            client,
            endpoint,
            api_key: key,
        })
    }

    pub async fn evaluate(
        &self,
        state: Value,
        questions: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let payload = json!({
            "model": "jev-latest",
            "state": state,
            "questions": questions
        });

        let resp = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?
            .json::<Value>()
            .await?;

        Ok(resp)
    }
}
```

#### 4.3 High-Level Evaluator Interface
Create `crates/codegen/xai-grok-shell/src/judgment/evaluator.rs`:

```rust
use super::client::JevClient;
use serde_json::json;
use tracing::warn;

#[derive(Clone)]
pub struct JudgmentEvaluator {
    client: JevClient,
    safety_threshold: f32,
}

impl JudgmentEvaluator {
    pub fn new(client: JevClient, safety_threshold: f32) -> Self {
        Self { client, safety_threshold }
    }

    /// Subsystem 1: Dynamic Reasoning Classification.
    /// `offered` is the active model's thinking menu. `None` = keep the already-configured effort.
    pub async fn classify_reasoning(
        &self,
        prompt: &str,
        subagent_type: Option<&str>,
        offered: &[ReasoningEffort],
    ) -> Option<&'static str> {
        if offered.is_empty() {
            return None;
        }
        if let Some("explore") = subagent_type {
            return cheapest_offered_effort(offered);
        }

        let state = json!({ "prompt": prompt, "subagent_type": subagent_type });
        let questions = json!({
            "complexity": {
                "type": "score",
                "prompt": "Evaluate cognitive difficulty. 0.0 = trivial syntax/minor edit/lookup, 1.0 = deep algorithmic reasoning, subtle deadlock, or architectural refactor."
            }
        });

        match self.client.evaluate(state, questions).await {
            Ok(res) => {
                // Live System One shape: {"answers":{"<id>":{"type":"score","score":1.5}}}
                // A `score` answer is the probability-weighted mean of criterion indices;
                // divide by 3.0 to land on the 0.0–1.0 bands below.
                let score = res["answers"]["complexity"]["score"].as_f64()? / 3.0;
                map_score_onto_offered_efforts(score, offered)
            }
            Err(e) => {
                warn!("Jev dynamic thinking evaluation failed, keeping configured effort: {e}");
                None
            }
        }
    }

    /// Subsystem 2: Output Distillation Check
    pub async fn has_actionable_errors(&self, output_sample: &str) -> bool {
        let state = json!({ "output_sample": output_sample.chars().take(2500).collect::<String>() });
        let questions = json!({
            "has_errors": {
                "type": "noul",
                "prompt": "Does this CLI/test output contain active compiler errors, stack traces, or failed unit assertions requiring developer action?"
            }
        });

        match self.client.evaluate(state, questions).await {
            Ok(res) => res["answers"]["has_errors"]["noul"].as_f64().unwrap_or(1.0) > 0.50,
            Err(e) => {
                warn!("Jev output distillation check failed, failing open: {e}");
                true // Fail-open: keep full output
            }
        }
    }

    /// Subsystem 3: Tournament Patch Quality Scorer
    pub async fn score_patch(&self, task: &str, diff: &str) -> f64 {
        let state = json!({
            "task": task,
            "diff": diff.chars().take(8000).collect::<String>()
        });
        let questions = json!({
            "quality": {
                "type": "score",
                "prompt": "Score how cleanly and completely this git patch addresses the task requirements without regressions or incomplete stubs."
            }
        });

        match self.client.evaluate(state, questions).await {
            Ok(res) => res["answers"]["quality"]["score"].as_f64().map(|s| s / 3.0).unwrap_or(0.0),
            Err(e) => {
                warn!("Jev patch scoring failed: {e}");
                0.5 // Neutral score
            }
        }
    }

    /// Subsystem 4: Dead-End Verification
    pub async fn verify_dead_end(&self, summary: &str) -> bool {
        let state = json!({ "turn_summary": summary });
        let questions = json!({
            "is_abandoned": {
                "type": "noul",
                "prompt": "Did this series of turns represent an abandoned debugging hypothesis that was fully reverted and provides no forward utility?"
            }
        });

        match self.client.evaluate(state, questions).await {
            Ok(res) => res["answers"]["is_abandoned"]["noul"].as_f64().unwrap_or(0.0) > 0.75,
            Err(_) => false,
        }
    }

    /// Subsystem 5: Pre-Execution Safety Gating
    pub async fn is_safe_tool(&self, tool: &str, input: &str) -> bool {
        let state = json!({ "tool": tool, "input": input });
        let questions = json!({
            "is_destructive": {
                "type": "noul",
                "prompt": "Is this command or edit destructive, irreversible, dropping state, or deleting uncommitted work outside the workspace?"
            }
        });

        match self.client.evaluate(state, questions).await {
            Ok(res) => {
                let risk = res["answers"]["is_destructive"]["noul"].as_f64().unwrap_or(1.0);
                risk < (self.safety_threshold as f64)
            }
            Err(_) => false, // Fail-safe: require manual user approval
        }
    }
}
```

---

## 5. Subsystem Implementation Details

### Subsystem 1: Dynamic Parent Reasoning Effort
* **File:** `crates/codegen/xai-grok-shell/src/session/turn.rs` (or `turn_runner.rs`)
* **Behavior:** When assembling the inference parameters for the current turn:
  ```rust
  if let Some(ref j_cfg) = session.config.judgment {
      if j_cfg.enabled && j_cfg.dynamic_thinking {
          if let Some(ref evaluator) = session.judgment_evaluator {
              if let Some(effort) = evaluator.classify_reasoning(&turn_prompt, None).await {
                  completion_params.reasoning_effort = Some(effort.to_string());
              }
          }
      }
  }
  ```

---

### Subsystem 2: Dynamic Subagent Reasoning Effort
* **Files:** `crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs` (`apply_dynamic_subagent_effort`) and `handle_request.rs`. `xai-grok-tools` is unmodified.
* **When:** After the child's model is final (unknown-model fallback and resume pin done) and after an explicit spawn `reasoning_effort` has been applied. Child sessions must **not** re-classify on later turns: `apply_dynamic_reasoning_effort` returns immediately when `startup_hints.is_subagent`.
* **Behavior:**
  1. If `effective_runtime.reasoning_effort` is already `Some`, skip Jev entirely (explicit / role / persona / definition outranks Jev by not calling it).
  2. If the task prompt is empty or whitespace, keep the inherited parent effort.
  3. Else if `[judgment] enabled + dynamic_subagent_thinking` and a hook exists, classify against **that child's model menu** (`ModelsManager::offered_reasoning_effort_values`).
     * Empty menu (model has no thinking settings) → keep inherited effort, no network call.
     * `explore` → cheapest offered level, no network call.
     * Otherwise map the 0–1 complexity score onto equal-width bands of the offered list, cheapest first (Grok 4.6 → `low`/`medium`/`high`/`xhigh`; Grok 4.5 → `low`/`medium`/`high`, never `xhigh`).
     * `None` (timeout / HTTP / malformed / missing score) keeps the inherited effort. Never substitute `"medium"`.
  4. Apply a `Some` verdict through `ModelsManager::apply_supported_effort` so a model that does not support reasoning effort ignores it.

---

### Subsystem 3: Subagent Tournament Pruning
* **Status:** Primitive only. This repository has no parallel-candidate tournament executor. `rank_candidates` / `tournament_resolution_line` live in `crates/codegen/xai-grok-shell/src/judgment/tournament.rs` and are unit-tested. `tournament_pruning_enabled()` has no production call site. `admission.rs` is concurrency admission, not candidate merge.
* **When an executor exists:** Before tearing down parallel isolated git worktrees:
  1. Capture `git diff <base_sha>..HEAD` within each active worktree directory.
  2. Score each candidate diff via `evaluator.score_patch(&task_prompt, &diff).await`.
  3. Sort candidates. Merge the highest-scoring candidate.
  4. Discard transcripts and tool outputs of losing candidates.
  5. Inject into the parent session history only a synthetic 2-line resolution:
     ```text
     [Tournament Manager: 3 subagents executed. Candidate B selected (Score: 0.94). Candidates A and C pruned.]
     ```

---

### Subsystem 4: Tool Output Distillation
* **File:** `crates/codegen/xai-grok-shell/src/session/acp_session_impl/tool_dispatch.rs`
* **Behavior:** In `PostToolUse` for shell executions:
  1. Count lines of `stdout + stderr`. If $\le \text{distill\_line\_threshold}$, append raw text normally.
  2. If $> \text{distill\_line\_threshold}$, call `evaluator.has_actionable_errors(&stdout).await`.
  3. If `false` (clean build/passing tests):
     * Retain the first 5 lines and last 5 lines.
     * Replace intermediate text in the transcript with an omission count. Full output is **not** written to a side log (there is no retrieval path); re-run a narrower command if the middle is needed.
       ```text
       [90 lines omitted; output had no actionable errors. Re-run with a narrower command if you need the middle.]
       ```

---

### Subsystem 5: Contiguous Tail Dead-End Scrubbing
* **File:** `crates/codegen/xai-grok-shell/src/session/turn.rs`
* **Behavior:**
  1. Inspect the last executed tool call in the active turn.
  2. If the tool executed an explicit revert command (`git checkout -- .`, `git restore`, `git reset --hard`):
     * Assemble a text summary of the immediate prior 2 turns.
     * Call `evaluator.verify_dead_end(&summary).await`.
     * Re-read the conversation and re-plan. If the length or the plan changed during the Jev call, **abort** — do not chop a stale item count off the new end.
     * If the re-plan matches, slice off that contiguous tail.
     * Insert a synthetic checkpoint:
       ```text
       [System: Reverted failed hypothesis in file X; turns pruned to preserve context.]
       ```
  3. *Constraint:* **Never prune turns prior to the immediately preceding sequence** to ensure the prefix prompt cache remains valid.

---

### Subsystem 6: Pre-Execution Safety Gating
* **File:** `crates/codegen/xai-grok-shell/src/session/acp_session_impl/tool_dispatch.rs`
* **Behavior:** Before routing to the interactive approval prompt:
  ```rust
  if config.judgment.as_ref().map_or(false, |j| j.enabled && j.gate_tools) {
      if let Some(ref evaluator) = ctx.judgment_evaluator {
          if evaluator.is_safe_tool(&tool_name, &tool_input_str).await {
              // Auto-approve benign operation without user prompt
              return execute_tool_direct(ctx, tool_name, args).await;
          }
      }
  }
  ```

---

## 6. Implementation Checklist & Verification Gates

### Phase 1: Configuration & Schema
- [ ] Add `JudgmentConfig` to `crates/codegen/xai-grok-config/src/judgment.rs`.
- [ ] Export `JudgmentConfig` in `xai-grok-config/src/lib.rs`.
- [ ] Verify: `cargo check -p xai-grok-config`
- [ ] Verify: `cargo test -p xai-grok-config`

### Phase 2: Client & Engine
- [ ] Add `reqwest` to `crates/codegen/xai-grok-shell/Cargo.toml`.
- [ ] Implement `JevClient` and `JudgmentEvaluator` in `crates/codegen/xai-grok-shell/src/judgment/`.
- [ ] Add unit test verifying graceful failure and fallback when endpoint is unreachable.
- [ ] Verify: `cargo check -p xai-grok-shell`
- [ ] Verify: `cargo test -p xai-grok-shell`

### Phase 3: Token Reduction Hooks
- [ ] Wire dynamic reasoning into parent `turn.rs`.
- [ ] Wire dynamic subagent reasoning into `task/spawn.rs`.
- [ ] Wire output distillation into `tool_dispatch.rs`.
- [ ] Wire tournament pruning into subagent merge coordinator.
- [ ] Wire tail dead-end scrubber into `turn.rs`.
- [ ] Verify: `cargo check -p xai-grok-tools`

### Phase 4: Safety Gating
- [ ] Wire tool safety check into `tool_dispatch.rs`.
- [ ] Verify that destructive commands (`rm -rf`, `git reset --hard`) still trigger interactive approval prompts.

### Phase 5: End-to-End Build & Validation
- [ ] Build binary: `cargo check -p xai-grok-pager-bin`
- [ ] Run with empty config: confirm identical behavior to vanilla upstream.
- [ ] Run with `[judgment]` enabled: confirm dynamic thinking and output compression in active session.