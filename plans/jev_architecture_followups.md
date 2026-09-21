# JEV Architecture Follow-ups Implementation Plan

> **For agentic workers:** Implement task-by-task with TDD. Do not edit the root `Cargo.toml` or `Cargo.lock`.

**Goal:** Close the five architecture-review follow-ups so Settings, session, subagent spawn, dead-end pruning, and reasoning classification agree with each other and with the live TypeSafe API.

**Architecture:** Keep the gated sidecar. Change only the seams that currently lie: schema defaults vs Settings UI, two `[judgment]` loaders, prune-by-stale-count, `classify_reasoning` substituting `"medium"`, and the stale spec.

**Tech Stack:** Rust workspace crates `xai-grok-config`, `xai-grok-shell`, `xai-grok-pager`. TypeSafe System One at `https://api.typesafe.ai/v1/systemone`.

## Global Constraints

- Root `Cargo.toml` / `Cargo.lock` are read-only.
- Zero-regression: absent `[judgment]`, `enabled = false`, or no credential ⇒ no Jev work.
- Overlay-free load: `GROK_CONFIG` must not spend a TypeSafe credential.
- Safety gating still fails closed; every other subsystem fails open to vanilla.
- Wire tag `dynamic_reasoning_effort` is frozen.
- `xai-grok-tools` stays unmodified.

## Decisions (review of each follow-up)

### 1. Master-enable vs UI lever defaults

**Problem:** Settings shows `gate_tools` / `dynamic_thinking` / `distill_outputs` as ON when unset. `JudgmentConfig` defaults those bools to `false`. Enabling the master switch via the modal does `get_or_insert_with(Default)` and writes `false`, so the first opt-in creates a hook that does nothing.

**Rejected:** Change the modal to show OFF. That contradicts “toggling the master switch alone produces the documented behavior.”

**Rejected:** `Option<bool>` + skip `None`. Correct, but a wider schema change than needed; `bool` with serde default `true` already makes an omitted key mean ON.

**Chosen:** The three UI-exposed levers default **true** in both `Default` and serde (`default_true`). Aggressive, non-UI levers (`dynamic_subagent_thinking`, `tournament_pruning`, `prune_dead_ends`) stay **false**. Always serialize the UI levers so a later toggle ON can overwrite a stored `false` through `merge_section`.

`enabled` stays default `false`. Section absent is still vanilla.

### 2. One `[judgment]` resolver

**Problem:** The session hook reads overlay-free disk (`judgment_config_from_disk`). Subagent spawn builds a hook from `self.cfg.borrow().judgment`, which can include the `GROK_CONFIG` overlay.

**Chosen:** One helper `resolve_judgment_hook()` in `agent/judgment_config.rs` that is `JudgmentHook::from_config(judgment_config_from_disk().as_ref())`. Session `OnceLock` and subagent spawn both call it. Share the loader, not a process-wide `static` (tests would leak). Two HTTP clients in one process is acceptable; config identity is the invariant.

### 3. Dead-end prune after Jev

**Problem:** `maybe_prune_dead_end_tail` plans `remove_items`, awaits Jev, re-reads, then truncates that many items off the **new** end.

**Chosen:** Pure helper `confirm_dead_end_prune(original_len, original_remove, new_len, new_plan) -> Option<usize>`. After the verdict, re-plan the fresh conversation. If length changed or the new plan differs, abort (leave history). Never chop a stale count.

### 4. `classify_reasoning` → `Option`

**Problem:** Failure returns `"medium"`, and call sites apply it, so an outage can raise a `low` user setting to `medium`.

**Chosen:** Return `Option<&'static str>`. `Some` only for a mapped score or the pinned `explore` → `minimal`. `None` on timeout/HTTP/malformed/missing score, with `warn!`. Call sites apply and emit `DynamicReasoningEffort` only on `Some` that parses. Configured effort stays put.

### 5. Spec catch-up

Update `plans/typesafe_jev_token_optimization_specification.md` in place: live `answers.<id>` shape, hand-written `Default`, UI-lever defaults, UI-key map, overlay-free shared resolver, tournament primitive-only, distill-without-log (actual behavior), `Option` classification, re-plan-after-Jev.

---

### Task 1: UI lever defaults

**Files:**
- Modify: `crates/codegen/xai-grok-config/src/judgment.rs`
- Modify: `crates/codegen/xai-grok-config/src/judgment_tests.rs`
- Modify: `crates/codegen/xai-grok-shell/src/util/config/persist_tests.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/judgment_config.rs`
- Modify: `crates/codegen/xai-grok-shell/src/judgment/hook_tests.rs`

- [x] Failing test: `[judgment] enabled = true` (no other keys) parses with the three UI levers true; `Default` matches.
- [x] Failing persist test: `judgment_mut` + `enabled = true` on an absent section does not leave those levers false after merge/reparse.
- [x] Implementation: `default_true()` on `dynamic_thinking`, `distill_outputs`, `gate_tools`; `Default` matches.
- [x] Fix hook test that assumed Default levers are off.

### Task 2: Shared resolver

**Files:**
- Modify: `crates/codegen/xai-grok-shell/src/agent/judgment_config.rs`
- Modify: `crates/codegen/xai-grok-shell/src/session/acp_session_impl/sampler_turn.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/mvp_agent/subagent_spawn.rs`

- [x] Add `resolve_judgment_hook() -> Option<JudgmentHook>`.
- [x] Session `OnceLock` and subagent spawn both call it.
- [x] Test: `resolve_judgment_hook` is `from_config(judgment_config_from_disk())` (documented by the helper; unit tests stay on `from_config` / TOML parse).

### Task 3: Re-plan dead-end tail

**Files:**
- Modify: `crates/codegen/xai-grok-shell/src/judgment/subsystems.rs`
- Modify: `crates/codegen/xai-grok-shell/src/judgment/subsystems_tests.rs`
- Modify: `crates/codegen/xai-grok-shell/src/session/acp_session_impl/sampler_turn.rs`

- [x] Failing tests for `confirm_dead_end_prune`: same plan → `Some(remove)`; length change → `None`; plan change → `None`.
- [x] Implement helper; call it after the Jev verdict with a fresh `plan_dead_end_prune`.

### Task 4: Option classification

**Files:**
- Modify: `crates/codegen/xai-grok-shell/src/judgment/evaluator.rs`
- Modify: `crates/codegen/xai-grok-shell/src/judgment/evaluator_tests.rs`
- Modify: `crates/codegen/xai-grok-shell/src/session/acp_session_impl/sampler_turn.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs`

- [x] Failing tests: unreachable / HTTP error / missing answer → `None`; explore → `Some("low")`; mapped scores → `Some(...)`.
- [x] Change return type; call sites `if let Some(effort)`.
- [x] Drop `FALLBACK_REASONING_EFFORT`.

### Task 5: Spec

**Files:**
- Modify: `plans/typesafe_jev_token_optimization_specification.md`

- [x] Rewrite the stale sections listed in Decision 5. Keep the zero-regression mandate.

## Verification

```
cargo test -p xai-grok-config --lib judgment
cargo test -p xai-grok-shell --lib judgment
cargo test -p xai-grok-shell --lib util::config::persist_tests
cargo test -p xai-grok-shell --lib agent::judgment_config
```

Do not claim complete until those pass.
