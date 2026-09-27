# 25 — Feature pinning table (enterprise)

**Mirror of `FEATURES` in `crates/codegen/xai-grok-config-types/src/registry.rs`.** Every feature key
must appear below. The check is
`crates/codegen/xai-grok-pager/tests/registered_features_are_documented.rs`: a feature added to the
registry with no row here fails this crate's test build. That is deliberate — the registry is the
source of truth and this table is the hand-maintained operator mirror, so the mirror needs a
compile-time check or it silently rots.

The authoritative description of what a feature *does* is its entry in the registry, not this file.
What an operator needs from here is **where it is pinned** and **what happens when the tiers
disagree**.

## Where a feature can be pinned

`config.toml` uses the `path` column below; the process tier uses the variable listed in
`22-environment-variables.md`. The remote tier is the *feature flag* pin.

Precedence, highest tier first, as implemented by `resolve_bool_flag` in
`crates/codegen/xai-grok-config-types/src/flags.rs`:

```
requirement → cli → env → config → managed → feature flag (remote) → default
```

The counter-intuitive part, and the reason operators get this wrong: **the environment beats
`config.toml`**, and the remote pin beats almost nothing. Only `requirement` (a hard product
requirement) and an explicit CLI flag outrank the process environment.

| Feature key | `config.toml` path | Default | Remotely pinnable |
|---|---|---|---|
| `active_agent_messages` | `features.active_agent_messages` | off | yes |
| `ask_user_question` | `features.ask_user_question` | on | yes |
| `auto_wake` | `features.auto_wake` | on | yes |
| `backend_tools` | `features.backend_tools` | on | no — config only |
| `cancel_rewind` | `features.cancel_rewind` | on | yes |
| `compaction_verbatim_input` | `features.compaction_verbatim_input` | on | yes |
| `dock` | `features.dock` | off | yes |
| `feedback` | `features.feedback` | on | yes |
| `feedback_trace_card` | `features.feedback_trace_card` | off | yes |
| `lsp_tools` | `features.lsp_tools` | off | yes |
| `session_recap` | `features.session_recap` | on | yes |
| `session_search` | `features.session_search` | on | yes |
| `subagent_model_inheritance` | `features.subagent_model_inheritance` | off | yes |
| `subagent_worktree_snapshot` | `features.subagent_worktree_snapshot` | off | yes |
| `terminal_theme` | `features.terminal_theme` | off | yes |
| `turn_summary` | `features.turn_summary` | on | yes |
| `two_pass_compaction` | `features.two_pass_compaction` | on | yes |
| `voice_mode` | `features.voice_mode` | on | yes |
| `web_fetch` | `features.web_fetch` | off | yes |
| `write_file` | `features.write_file` | on | yes |

`backend_tools` is the one feature with no remote tier. That is a deliberate edit, not an omission:
adding a remote projection is a change to the product's rollout story, not a documentation task.

## When a feature appears stuck

1. The key is absent from `config.toml` and no variable is exported → the default applies.
2. The key is set in `config.toml` but the variable of the same name is also exported → **the variable
   wins**. This is the usual cause of "I set it to false and it is still on": the shell export fills a
   key you thought you had overridden.
3. A managed policy sets it → managed still loses to the environment and to `config.toml`. It only
   wins where both are absent.
4. A remote pin disagrees with everything local → the remote pin loses to the environment,
   `config.toml` and managed. It is the weakest tier above the default.
5. A `requirement` exists for this build → nothing local can change it, including the environment.

`UNMIRRORED_BOOLEAN_FEATURES` in `xai-grok-shell/src/agent/config.rs` holds four boolean config paths
that are deliberately **not** in the feature registry — `campaigns`, `remember_mode`,
`remote_fetch`, `zdr_access_enabled`. They have no row above and no environment tier; set them in
`config.toml` only.
