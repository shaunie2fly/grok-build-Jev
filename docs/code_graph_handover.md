# grok-build-Jev — Code Graph Thin Client: Validation Handover

**Audience:** an independent agent asked to *falsify* this work, not to re-run it and nod.
**Repo:** `/mnt/data/repos/grok-build-Jev` · **Date:** 2026-09-27
**Body of work:** `34d26652..c680d3c7` (8 commits, +2090/−28, 20 files) — authored by the grok
agent, merged into `main`, unpushed. **Follow-ups:** `6ba049a3`, `239c0091`, `a852a525`.
**Sibling document:** `judgment_validation_handover.md` covers `a616779e` and is a different body
of work. Read its §13 Postscript for the boundary.

The author's stance: every claim below is labelled *measured*, *inspected*, or *inferred*, and §6
lists what is **not** verified. Treat §6 as the most important section.

---

## 0. How to approach this validation

The work claims four things:

1. Three new read-only tools (`search_symbols`, `trace_calls`, `blast_radius`) reach the existing
   codebase-memory MCP server with bounded, non-blocking, fail-open-to-grep behaviour.
2. The new `ToolKind::CodeGraph` is bound into every closed set that enumerates tool kinds.
3. The capability grant matches what the tools can actually do.
4. The token cost of a symbol lookup is far below a saturated grep.

**Do not validate by re-running the author's tests alone.** §4 has the live probes that decide
whether the client and the real server actually agree, and §5 has the limits the author knows about
and did not fix. If §4's probes fail for you, the arg contract has drifted from the server and
that outranks everything else here.

---

## 1. Scope of completed work

| # | Commit | Subject |
|---|---|---|
| 1 | `34d26652` | classify CodeGraph as read-only code intelligence |
| 2 | `e15a1cc5` | include code_graph in the tool meta schema |
| 3 | `26abf82b` | select a codebase-memory project and cap graph text |
| 4 | `0cc58fc9` | dispatch capped codebase-memory queries through MCP |
| 5 | `219e2c30` | treat unknown-project only as an error, not as graph text |
| 6 | `914b2a22` | add native codebase-memory symbol, call, and blast-radius tools |
| 7 | `824be105` | report token cost before and after the code graph tools |
| 8 | `c680d3c7` | offer code graph tools on plan-without-subagents and ask-user |

New code: `crates/codegen/xai-grok-tools/src/implementations/grok_build/code_graph/`
(`client.rs` dispatch, `select.rs` project choice, `limits.rs` caps, `tools.rs` model-facing
surface, `token_report.rs` measurement).

### Repository state

```
main         = a852a525  (12 commits ahead of origin/main, UNPUSHED)
origin/main  = 80c13220  (contains the judgment body, not this one)
code graph   = 34d26652 … c680d3c7, then 6ba049a3 / 239c0091 / a852a525 (follow-ups)
working tree: clean
installed    : ~/.grok/bin/grok → grok-1.0.41-jev-c680d3c7  (predates the follow-ups)
```

**Check 1.1 — the body and its follow-ups are real and correctly ordered**

```sh
cd /mnt/data/repos/grok-build-Jev
git log --oneline 80c13220..HEAD | cat          # expect 11 commits, 8 body + 3 follow-up
git merge-base --is-ancestor c680d3c7 HEAD && echo "body is in HEAD"
git status --porcelain                          # expect EMPTY
```

---

## 2. What the tools do

`search_symbols` → MCP `search_graph`; `trace_calls` → `trace_path`; `blast_radius` →
`detect_changes`. Each resolves a project name first (`list_projects` → the covering root with the
most path components, then the larger index), caches it per cwd for 60 s, and caps the reply.

| Tool | Row cap | Byte cap | Depth | Notes |
|---|---|---|---|---|
| `search_symbols` | 15 | 2 000 | — | requires `name_pattern` or `query` |
| `trace_calls` | 30 | 3 000 | 1–3, default 2 | `include_tests: false` |
| `blast_radius` | 40 | 4 000 | 1–3, default 2 | optional `since` git ref |

Every failure returns a sentence that ends in "Use grep", never an exception the model must
interpret: `NOT_CONNECTED`, `NO_INDEX`, `QUERY_FAILED`, `QUERY_TIMEOUT`, plus a truncation hint.

---

## 3. Checkable claims

### Check 3.1 — the suite passes and the count is explained

```sh
cargo test -p xai-grok-tools --lib code_graph
# expect: 35 passed; 0 failed
```

The count was 35 before the follow-ups and is 35 after: `239c0091` deleted
`caps_and_fail_open_sentences_match_the_plan` (it compared each constant to its own literal and
could not fail) and added `the_live_unknown_project_body_is_still_recognised` (it holds the exact
body a live server returns).

### Check 3.2 — hygiene

```sh
cargo fmt -p xai-grok-tools -p xai-grok-workspace -p xai-grok-agent -- --check   # exit 0
cargo clippy -p xai-grok-tools -p xai-grok-workspace -p xai-grok-agent --lib -- -D warnings
```

> **This gate was red when the body landed.** `cargo fmt --check` failed with one diff — the import
> order in `token_report.rs`. Fixed in `6ba049a3`. If you find another diff, it is yours, not the
> body's.

Clippy emits exactly one warning, in a **build script**:
`tokio::process::Command::spawn` at `clippy.toml:53` — pre-existing, unrelated, and already
documented in `judgment_validation_handover.md` §3.4.

### Check 3.3 — the closed set is bound everywhere

Adding a `ToolKind` variant touches every enumeration. All of these were updated in the body and
must stay in step:

```sh
grep -rn "CodeGraph" --include=*.rs --include=*.json crates | sed 's/:.*//' | sort -u
```

Expect all of: `types/tool.rs` (the variant), `tool_taxonomy.rs` (presentation name + `is_read_only`),
`xai-grok-workspace/src/capability.rs` (`ALL_TOOL_KINDS` + the inspect-class arm of `kind_allowed`),
`.../permission/types.rs` (`AccessKind::Read(None)`), `task/types.rs` (all four
`SubagentCapabilityMode` arms), `media_gen_limits.rs` (+ the `VARIANT_COUNT` 33→34 assertion),
`normalization.rs`, `registry/types.rs`, `types/tool_io.rs`, `schema/tool_meta.schema.json`.

`capability.rs:103` has a compile-time `const _: () = assert!(ALL_TOOL_KINDS.len() ==
ToolKind::VARIANT_COUNT)`; a new variant cannot be added without forcing a triage decision. The
`ToolKind` wire format is an **open** set (`#[serde(other)]`), so a client that does not know
`code_graph` silently reads it as `other`.

### Check 3.4 — the permission boundary is a delegation, not a copy

`xai-grok-shell/src/session/acp_session_impl/tool_calls.rs` used to carry its own inline
"read-only ⇒ skip write approval" list. It now calls `tool_taxonomy::skips_write_approval`, which
after `a852a525` delegates to `ToolKind::is_read_only`.

**The mutation that matters:** add a new read-only kind to `is_read_only` and confirm
`skips_write_approval` follows it automatically. Before `a852a525` it would not have — the two were
independent 13-item matches with nothing forcing them to agree, and the failure mode was silent.

Set equality with the old inline list was verified member by member: the old list is exactly
`is_read_only()` plus `CodeGraph`, so **no pre-existing tool changed behaviour**.

### Check 3.5 — the tools reach the real toolsets

```sh
cargo test -p xai-grok-agent --lib code_graph            # expect 1 passed
cargo test -p xai-grok-workspace --lib capability        # expect 11 passed
```

Presence is asserted on `grok-build`, `grok-build-concise`, `grok-build-plan`, `explore`, `plan`,
`orchestrator`, the hashline toolset, `grok-build-plan-no-subagents` and `grok-build-ask-user`;
absence on `grok-computer` and `codex`.

> **Gap in the check:** the negative leg only asserts `search_symbols` is absent from
> `grok-computer`/`codex`. `trace_calls` and `blast_radius` are not asserted absent. Verify by hand
> before trusting it.

---

## 4. Live verification — the part that decides the client/server contract

Everything above is offline. These probes ran against codebase-memory-mcp 0.10.2 with the exact
argument shapes `client.rs` sends. Re-run them; a failure here outranks the test suite.

**4.1 — project listing parses.** `list_projects` returns `name`, `root_path`, `nodes`,
`size_bytes`; `parse_projects` requires the first three and defaults the counts to 0.

**4.2 — duplicate indexes on one root are resolved deterministically.** This host carries **two**
indexes for `/mnt/data/repos/grok-build-Jev` (`mnt-data-repos-grok-build-Jev`, 133 655 nodes;
`grok-build-Jev`, 133 395). The ranking — path depth, then `nodes`, then `size_bytes`, then name —
picks the larger. Any host that re-indexes under a second name hits this path, so it is not
hypothetical.

**4.3 — every tool accepts the client's arg shape.**
`search_graph {project, query, limit:15, offset:0, format:"tree", detail:"default"}`;
`trace_path {project, function_name, direction:"inbound", depth:2, limit:30, format:"tree",
mode:"calls", include_tests:false}`; `detect_changes {project, scope:"impact", direction:"inbound",
depth:2, limit:40, format:"tree", since:"HEAD~3"}`. All three returned data.

**4.4 — an unknown project comes back as an MCP *error* body**, not a success body:

```json
{"error":"project not found or not indexed",
 "hint":"Use list_projects to see all indexed projects, then pass it as the \"project\" argument.",
 "available_projects":[…],"count":11}
```

This is what `UNKNOWN_PROJECT_MARKERS` matches, and it is why the markers are checked on error
bodies only. `the_live_unknown_project_body_is_still_recognised` pins the exact string; if a server
upgrade rephrases it, that test fails instead of the markers silently falling through to
`QUERY_FAILED` and losing the single automatic retry.

**4.5 — no blocking call on the async path.** The client dispatches through
`use_tool::dispatch_mcp_tool` (an MCP call, no `std::process`/`reqwest::blocking`), wraps it in a
15 s `tokio::time::timeout`, and `select.rs` touches no filesystem at all. The only sync work is
string handling and a `tokio::sync::Mutex` around the project cache.

**4.6 — the server-name assumption is load-bearing and unverified in CI.**
`qualified()` builds `codebase-memory-mcp__<tool>`. If a user registers the server under a
different name, every call returns `NOT_CONNECTED`, which since `239c0091` at least names the
server. No test covers a real MCP registry lookup — the suite injects a fake `ToolDispatch`.

---

## 5. Known limits the author did not fix

1. **`blast_radius` truncation drops the module rollup.** A `detect_changes` at `limit=40` is
   ≈5.5 KB against a 4 000-byte cap, and the cap cuts from the end. The header survives — it
   reports `impacted_total` and `impacted_shown` — so the model is not misled about how much
   existed. What is lost is the trailing `impacted_omitted` / `impacted_modules` block.
   `search_graph` and `trace_path` are unaffected: both put their counts in the header.
2. **The MCP server name is hardcoded.** See 4.6.
3. **The project cache is process-global**, keyed by cwd with a 60 s TTL, and never pruned. Two
   sessions on the same cwd share the entry; a stale name is self-correcting via the retry path, and
   expiry is covered by `expired_project_cache_lists_again`. Unbounded growth is the only real cost.
4. **`is_not_connected` is broad** — it matches `ToolErrorKind::NotFound` plus two substrings. A
   different "not found" would be reported as "not connected". Fail-safe, but imprecise.
5. **No in-agent run has ever exercised these tools.** See §6.1.

---

## 6. Not verified / open items

Do not report these as validated.

1. **No end-to-end run through a real `grok` session.** All 35 tests inject a fake `ToolDispatch`;
   §4's probes exercise the server, not the tool loop. Reproduce with a scratch home and a live
   `codebase-memory-mcp`: `search_symbols` a known symbol, `trace_calls` on it, `blast_radius` with
   `since=HEAD~1`. This is the only way to confirm or dismiss limit 5.1 empirically.
2. **Nothing is pushed.** `main` is 12 commits ahead of `origin/main`, and there is no tag for this
   body (the repo's only tag is `v1.0.38-jev`, on `050d560b`).
3. **`cargo check --workspace --all-targets` is red for a pre-existing reason**:
   `crates/codegen/xai-grok-pager/tests/registered_features_are_documented.rs` `include_str!`s
   `../docs/internal/25-enterprise.md` and `22-environment-variables.md`, and
   `crates/codegen/xai-grok-pager/docs/internal/` does not exist. No pager file appears in
   `80c13220..c680d3c7`. A green targeted suite does not mean the workspace builds.
4. **The installed binary predates the follow-ups.** `~/.grok/bin/grok` is `c680d3c7`; the three
   follow-up commits are not in it. `agent` still points at `grok-1.0.41-jev-a616779`.

---

## 7. The token-cost claim

Reproduce — the report prints with `--nocapture`:

```sh
cargo test -p xai-grok-tools --lib code_graph::token_report -- --nocapture
```

```
estimator: xai_token_estimation::estimate_tokens (bytes/4)
session: 8 turns; descriptions are resent every turn; lookups are 0, 1, or 3

row                          before   after   delta
grep content cap              10000     522    9478
trace cap                     10000     772    9228
blast cap                     10000    1022    8978
mcp discovery plus body        5628     522    5106
session 8 turns, 0 lookups        0    5728   overhead
session 8 turns, 1 lookup     10000    6250    3750
session 8 turns, 3 lookups    30000    7294   22706
```

Read it carefully. `before` is a **saturated** grep — the content cap — not a typical result; a grep
that matches two lines is already cheaper than the tool. The defensible claims are the `overhead`
row (three definitions resent over 8 turns cost 5728 tokens) and the crossover: one lookup in eight
turns saves 3750, three save 22 706, and a raw MCP discovery plus body costs 5628 versus 522.

`token_report.rs` is `#[cfg(test)]` since `a852a525`. It has no production caller and must not ship
in the library; the table above is its durable output.

---

## 8. Traps

**Trap 1 — a green suite is not a working tool.** The 35 tests never spawn a server, never resolve a
project against a real index, and never run inside an agent turn. §4 exists because of this.

**Trap 2 — duplicate indexes on one root are normal here.** Do not treat "which project name?" as a
theoretical question when validating selection logic.

**Trap 3 — the byte cap is byte-based.** `cap_text` cuts at a char boundary, so a multi-byte
symbol name cannot panic it — but it also means the cut lands mid-row. Compare `out.len()` against
`max_bytes + TRUNCATION_HINT.len()`, not against a hardcoded literal; the literal drifted once
already.

**Trap 4 — `ToolKind` is an open set.** A consumer that has never heard of `code_graph` reads it as
`other`, and `other` is denied in every capability mode except `All`. A tool that "does not appear"
on an old client is not evidence the kind is missing.
