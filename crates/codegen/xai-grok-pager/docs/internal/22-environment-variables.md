# 22 — Environment variables

**Mirror of `FEATURES` in `crates/codegen/xai-grok-config-types/src/registry.rs`.** Every feature's
environment variable must appear below; the check is
`crates/codegen/xai-grok-pager/tests/registered_features_are_documented.rs`, which fails the build of
this crate's tests when a variable is added to the registry and not to this table.

Set a variable to a boolean to override the built-in default for that process.

The environment is a **high** tier: it beats `config.toml`, managed policy and the remote pin, and
loses only to a product `requirement` or an explicit CLI flag. Full order in `25-enterprise.md`;
implementation in `resolve_bool_flag` (`crates/codegen/xai-grok-config-types/src/flags.rs`).

## Feature toggles

| Variable | Feature key | Default |
|---|---|---|
| `GROK_ACTIVE_AGENT_MESSAGES` | `active_agent_messages` | off |
| `GROK_ASK_USER_QUESTION` | `ask_user_question` | on |
| `GROK_AUTO_WAKE` | `auto_wake` | on |
| `GROK_BACKEND_SEARCH` | `backend_tools` | on |
| `GROK_CANCEL_REWIND` | `cancel_rewind` | on |
| `GROK_COMPACTION_VERBATIM_INPUT` | `compaction_verbatim_input` | on |
| `GROK_DOCK` | `dock` | off |
| `GROK_FEEDBACK_ENABLED` | `feedback` | on |
| `GROK_FEEDBACK_TRACE_CARD` | `feedback_trace_card` | off |
| `GROK_LSP_TOOLS` | `lsp_tools` | off |
| `GROK_SESSION_RECAP` | `session_recap` | on |
| `GROK_SESSION_SEARCH` | `session_search` | on |
| `GROK_SUBAGENT_MODEL_INHERITANCE` | `subagent_model_inheritance` | off |
| `GROK_SUBAGENT_WORKTREE_SNAPSHOT` | `subagent_worktree_snapshot` | off |
| `GROK_TERMINAL_THEME` | `terminal_theme` | off |
| `GROK_TURN_SUMMARY` | `turn_summary` | on |
| `GROK_TWO_PASS_COMPACTION` | `two_pass_compaction` | on |
| `GROK_VOICE_MODE` | `voice_mode` | on |
| `GROK_WEB_FETCH` | `web_fetch` | off |
| `GROK_WRITE_FILE` | `write_file` | on |

## Process variables outside the feature registry

These are read directly by the binary, not through `FEATURES`, so the table above does not cover
them. They are listed because an operator debugging a deployment needs them in one place.

| Variable | Effect |
|---|---|
| `GROK_HOME` | Relocates the whole home: config, logs, sessions. Honoured by the config loader and by the unified log. The cleanest way to run an isolated test instance. |
| `GROK_VERSION` | Stamps the build. A fork build without it reports a bare upstream version and `deploy-fork.sh`'s tag gate refuses to install it. |
| `RUST_LOG` | Tracing verbosity. `tracing` output goes to stderr only; the structured records in `$GROK_HOME/logs/unified.jsonl` are unaffected. |
| `TYPESAFE_API_KEY`, `JEV_TYPESAFE_AI_KEY` | Credentials for the Jev judgment endpoint. **Secret.** Set them in a gitignored `.env` or the process environment; never in a committed file. |

A `.env` in the current working directory — or any parent — is loaded at startup and fills only
variables that are genuinely unset.
