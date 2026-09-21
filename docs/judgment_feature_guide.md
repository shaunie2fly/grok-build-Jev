# User Guide: The Judgment Feature Set in Grok Build
## Intelligent Token Optimization, Dynamic Reasoning, and Execution Gating

---

## 1. Executive Summary: Why Judgment?

As autonomous coding agents tackle complex development workflows, they encounter an inherent architectural friction: **frontier models are too powerful—and too expensive—for micro-decisions.**

State-of-the-art models like **Grok 4.6** and contemporary frontier reasoning models are designed for deep semantic understanding, multi-step problem solving, and complex software design. However, an agent session executes dozens of intermediate operations that do not require frontier intelligence:
- Classifying whether a task is trivial or architectural to choose a reasoning budget.
- Scanning a 600-line build output to determine if it passed or failed.
- Deciding whether a read-only terminal command (`ls`, `git status`) requires manual user approval.
- Evaluating whether an abandoned debugging attempt should remain in the active context window.

Using the primary frontier model to evaluate these micro-decisions creates three acute penalties:
1. **Context Bloat & Compounding Costs:** Every raw compiler output, repetitive tool transcript, and dead-end investigation accumulates in the prompt. In long sessions, context easily grows to 150k–300k+ tokens. At this scale, every single follow-up message costs **$0.05–$0.25+ per turn** merely to re-read history.
2. **Reasoning Inefficiency & Latency:** Spending 10,000+ "thinking tokens" on a simple typo fix or a one-line helper function adds 15–30 seconds of unnecessary latency and drains usage balance rapidly.
3. **Approval Fatigue:** Interrupting the developer to approve harmless commands (`cargo check`, `git diff`) creates friction, causing users to either enable dangerous `yolo` modes or abandon automated workflows.

### The Solution: A Two-System Architecture (System 1 + System 2)

Inspired by cognitive science (Kahneman’s *Thinking, Fast and Slow*), Grok Build introduces the **Judgment Feature Set** powered by **TypeSafe Jev**:

```
                       ┌────────────────────────────────────────────────────────┐
                       │                     Grok Build Session                 │
                       └───────────────────────────┬────────────────────────────┘
                                                   │
                         ┌─────────────────────────┴──────────────────────────┐
                         ▼                                                    ▼
             [System 1: Fast Judgment]                             [System 2: Deep Agent]
                  TypeSafe Jev                                           xAI Grok
          ───────────────────────────────                       ───────────────────────────
          • Sub-300ms latency                                   • Frontier generative model
          • Non-generative (discriminator)                      • Multi-step synthesis & reasoning
          • Micro-cost (~$0.0001/call)                          • High token cost (~$0.01-$0.25/call)
          • Classifies, scores, gates                           • Writes code, plans architecture
```

By delegating classification, log distillation, safety gating, and context curation to a fast, non-generative System 1 model, Grok Build preserves your frontier token budget and prompt-cache lifetime for where it matters most: **writing clean, working software.**

---

## 2. Core Subsystems: What Each Feature Does

The Judgment suite consists of five specialized subsystems operating across the lifecycle of each agent turn.

```
                  ┌──────────────────────────────────────────────────┐
                  │                 User Turn Starts                 │
                  └─────────────────────────┬────────────────────────┘
                                            │
                                            ▼
           1. Dynamic Thinking      [Classify Prompt Complexity]
                                            │
                                            ▼
                                  [Frontier Model Samples]
                                            │
                                            ▼
           2. Safety Gating         [Evaluate Proposed Tool Calls]
                                            │
                                            ▼
                                   [Execute Tool Call]
                                            │
                                            ▼
           3. Output Distillation   [Filter Raw Terminal/Test Output]
                                            │
                                            ▼
           4. Dead-End Pruning      [Detect Reverted Spikes at Tail]
                                            │
                                            ▼
                                    [Append to Context]
```

---

### Feature 1: Dynamic Reasoning Effort Allocation

#### The Problem
Modern frontier models offer controllable reasoning effort (e.g., `none`, `low`, `medium`, `high`, `xhigh`). Leaving reasoning pinned to `high` ensures difficult algorithmic bugs are solved, but wastes thousands of tokens on simple questions, formatting tasks, and file searches. Conversely, setting reasoning to `low` causes the agent to fail on complex refactors.

#### What Judgment Does
Before the frontier model samples a response, Judgment sends the user's prompt and active subagent role to Jev:
- Jev evaluates cognitive difficulty across a four-tier rubric (trivial $\rightarrow$ simple helper $\rightarrow$ multi-file refactor $\rightarrow$ architectural change) and returns a normalized score ($0.0 \dots 1.0$) with statistical confidence.
- **Per-Model Capability Alignment:** Rather than hardcoding fixed labels, Judgment inspects the active model's offered reasoning choices (such as Grok 4.6's `xhigh`/`high`/`medium`/`low` spectrum) and maps the score onto equal-width bands over that model's menu.
- **Subagent Optimizations:** Lightweight `explore` subagents are automatically pinned to the cheapest offered reasoning level without making a network call.
- **Fail-Safe Passthrough:** If the active model does not support reasoning effort, or if the network call times out, Judgment yields `None` and preserves the user's configured default effort.

---

### Feature 2: Context Output Distillation

#### The Problem
CLI commands (such as `cargo test`, `npm run build`, or `pytest`) frequently produce 200–1,000+ lines of text. Most of this output is repetitive status chatter (e.g., `Checking crate v0.1...`, `Compiling dependency...`). Injecting 500 lines of clean test passes into the conversation history consumes thousands of tokens on every subsequent turn.

#### What Judgment Does
When command output exceeds the configured threshold (default: 40 lines):
- Judgment extracts a bounded sample of the output and queries Jev's `has_errors` discriminator.
- Jev computes the probability (`noul`) that the output contains active compiler errors, assertion failures, or stack traces requiring developer attention.
- **Clean Output:** If no actionable errors are detected ($p < 0.50$), Judgment distills the text, retaining the first 5 lines (command initiation), an ellipsis indicating omitted lines, and the last 5 lines (final exit status).
- **Active Errors:** If errors or stack traces are detected ($p \ge 0.50$), the output is **preserved in full**.
- **Fail-Open Invariant:** If Jev is unreachable or encounters an error, the output is kept in full. Keeping text is always safe; dropping error context is not.

---

### Feature 3: Non-Destructive Tool Safety Gating

#### The Problem
To protect user systems, AI coding tools prompt users to approve bash commands and file changes. However, when an agent runs 20 consecutive read-only commands (`git status`, `find .`, `grep`, `cat config.toml`), frequent approval dialogs cause "approval fatigue," leading users to either blindly approve everything or enable risky unrestricted modes.

#### What Judgment Does
Before presenting an approval dialog, Judgment submits the tool name and command string to Jev's `is_destructive` evaluator:
- Evaluates whether the operation is destructive, drops database state, or deletes uncommitted work outside the repository workspace.
- **Safe Commands Auto-Approved:** Read-only inspections, test runs, and harmless commands with a risk score below the threshold (default: $0.20$) are approved automatically.
- **Destructive Commands Prompted:** Destructive operations (`rm -rf`, `git reset --hard`, `git clean -fdx`, `drop database`) receive high risk scores ($0.95+$) and **always** trigger the manual user approval dialog.
- **Fail-Closed Invariant:** Any error, timeout, or ambiguous classification immediately falls back to prompting the user for approval.

---

### Feature 4: Dead-End Hypothesis Pruning

#### The Problem
During complex debugging, an agent may pursue a hypothesis that proves incorrect, resulting in the agent issuing a `git revert` or rolling back changes. If left in the conversation history, these failed attempts clutter the context window and confuse the model on future turns. However, naive context truncation breaks provider-side **prompt caching**, which requires strict prefix stability.

#### What Judgment Does
- Slices only **contiguous trailing turns** that culminate in a net file reversion.
- Jev's `is_abandoned` discriminator verifies that the spike was completely abandoned and provides zero forward utility.
- Confirms that the conversation tail remained unchanged during the evaluation window (`confirm_dead_end_prune`).
- Removes the reverted intermediate turns while inserting a concise synthetic checkpoint summary, preserving prefix cache hit rates for preceding history.

---

### Feature 5: Tournament Patch Quality Scoring

#### The Problem
When running speculative execution or comparing multiple alternative implementations generated by competing subagents, selecting the best patch traditionally required another expensive frontier LLM turn.

#### What Judgment Does
- Evaluates competing git diffs against the task specification using Jev's `quality` scoring criteria.
- Assigns reproducible, normalized quality scores ($0.0 \dots 1.0$) to select the superior patch and discard substandard implementations before merging.

---

## 3. The Economics: Why System 1 Saves Dollars

To understand why this architecture is effective, compare the resource economics of frontier LLMs vs. TypeSafe Jev:

| Metric | Frontier Reasoning Model (e.g. Grok 4.6) | TypeSafe Jev (System 1) | Advantage |
|---|---|---|---|
| **Latency** | 2,000ms – 15,000ms | 150ms – 300ms | **10x – 50x faster** |
| **Cost per 1k tokens** | ~$0.003 – $0.015 (input) / $0.015 – $0.075 (output) | ~$0.0002 | **15x – 75x cheaper** |
| **Output Type** | Generative text / reasoning tokens | Deterministic probabilities & scores | **Zero hallucination** |
| **Prompt Cache Impact** | Degrades as raw logs enter context | Prevents context pollution | **Saves $0.05–$0.20/turn** |

### Real-World Example: A 20-Turn Debugging Session

Consider a standard 20-turn session involving 8 build/test commands and 12 code edits:

```
Without Judgment:
• Terminal outputs inject ~15,000 raw log tokens into context.
• Unpruned dead-end hypotheses add ~10,000 tokens.
• By Turn 15, prompt size reaches 180,000 tokens.
• Cumulative session cost: ~$4.80 - $7.20.

With Judgment:
• Output distillation compresses clean test passes by 85% (saving ~12,500 tokens).
• Dynamic reasoning selects 'low' or 'medium' on 11 non-complex turns (saving ~45,000 thinking tokens).
• Safe commands run with zero approval interruptions.
• Prompt size at Turn 15 stays bounded under 60,000 tokens.
• Cumulative session cost: ~$1.40 - $2.10 (60% - 70% cost reduction).
```

---

## 4. Safety, Privacy & Zero-Regression Guarantees

The Judgment feature set was built with enterprise-grade defensive invariants:

1. **Opt-In by Default:** If the `[judgment]` section is omitted from `~/.grok/config.toml` or `enabled = false`, Grok Build behaves identically to upstream vanilla code.
2. **Fail-Safe Fallbacks:**
   - *Output Distillation:* Fails **open** (retains full output if Jev is down).
   - *Safety Gating:* Fails **closed** (prompts the user if Jev is down).
   - *Reasoning Effort:* Falls back to user's configured default effort.
   - *Dead-End Pruning:* Declines to prune if confidence is low or tail changes.
3. **Strict Latency Budget:** All Jev calls are bounded by a configurable timeout (default: 2,500ms; typical: <300ms). A hung connection cannot stall the agent turn.
4. **Credential Isolation:** The judgment client strictly resolves credentials from the user's global disk configuration (`~/.grok/config.toml`). Project-local environment variables or cloned repository files (`GROK_CONFIG`) cannot inject endpoints or exfiltrate API keys.
5. **Non-Generative Privacy:** Jev receives state snippets (such as error outputs or command strings) and returns bounded numerical scores. It never writes, alters, or stores your source code.

---

## 5. Configuration & Settings Guide

The Judgment feature set is designed with zero-friction adoption in mind. Users can configure it interactively through the built-in terminal UI or declaratively via the global configuration file.

---

### 5.1 Quick Start (3 Steps)

1. **Obtain your TypeSafe API Key:** Get your key from the [TypeSafe Console](https://typesafe.ai).
2. **Provide your Credential:**
   - Either export it in your shell environment:
     ```bash
     export TYPESAFE_API_KEY="apikey_your_actual_key_here"
     ```
   - Or add it to `~/.grok/config.toml`:
     ```toml
     [judgment]
     api_key = "apikey_your_actual_key_here"
     ```
3. **Turn on the Master Switch:**
   - Press **`F2`** or **`Ctrl+,`** in `grok-build`, go to **Agent**, and toggle **TypeSafe Jev judgment** to `ON`.
   - Alternatively, add `enabled = true` under `[judgment]` in `~/.grok/config.toml`.
4. **Restart `grok-build`:**
   Because a session resolves its judgment hook at startup, restart `grok-build` to initialize the active hook.

---

### 5.2 Method 1: Interactive Settings Modal (In-App TUI)

Grok Build includes an interactive Settings Modal directly inside the terminal interface.

#### Opening the Settings Modal
You can open Settings from the Agent screen using any of the following methods:
- Press **`F2`** (Primary default key).
- Press **`Ctrl+,`** (or **`Super+,`** on macOS).
- Type the slash command **`/settings`** into the prompt and press Enter.
- Open the Command Palette (**`Ctrl+P`**) and type `settings`.

#### Navigating to Judgment Settings
1. In the left panel, scroll down to the **Agent** category (or press `Tab` to cycle categories).
2. You can also filter instantly: typing keywords like `judgment`, `jev`, `tokens`, `reasoning`, or `safety` will highlight the relevant rows.
3. Use the arrow keys (`↑` / `↓`) to highlight a setting and press **`Enter`** or **`Space`** to toggle.
4. Press **`Esc`** when finished to close the modal.

#### The 4 UI-Exposed Levers

| Setting Label | Registry Key | Default | Description |
|---|---|---|---|
| **TypeSafe Jev judgment** | `judgment.enabled` | `OFF` | Master switch. When OFF or absent, all Jev code is inactive and upstream vanilla behavior is preserved. |
| **Judgment safety gating** | `judgment.safety_gate_enabled` | `ON` | Lets Jev auto-approve commands scored as safe/non-destructive. Destructive or ambiguous commands still trigger manual approval. |
| **Judgment dynamic reasoning** | `judgment.dynamic_reasoning_enabled` | `ON` | Dynamically selects the frontier model's reasoning effort (`low`, `medium`, `high`, `xhigh`) per turn based on prompt complexity. |
| **Judgment output distillation** | `judgment.distillation_enabled` | `ON` | Replaces long, error-free command logs with clean head/tail excerpts to preserve prompt-cache space. Outputs with errors are kept in full. |

> [!NOTE]
> Every time you toggle a setting in the modal, a confirmation toast appears (e.g., `✓ TypeSafe Jev judgment: on (restart to apply)`). Sessions lock their judgment hook at initial creation, so restart `grok-build` to apply changes to new turns.

---

### 5.3 Method 2: Global Configuration File (`~/.grok/config.toml`)

Power users and headless environments can configure Judgment directly in the user's global configuration file located at `~/.grok/config.toml`.

#### Complete Reference Configuration

```toml
[judgment]
# ---------------------------------------------------------------------------
# Master Switch
# ---------------------------------------------------------------------------
# Enables the TypeSafe Jev subsystem. Default: false.
# When set to true, the UI levers (dynamic_thinking, distill_outputs, gate_tools)
# activate automatically unless explicitly set to false below.
enabled = true

# ---------------------------------------------------------------------------
# Credentials & Networking
# ---------------------------------------------------------------------------
# Explicit API key. Can be a literal string or an environment reference:
#   api_key = "apikey_2117e3bbe7da..."
#   api_key = "env:CUSTOM_TYPESAFE_VAR"
# If omitted or left blank, falls back to the TYPESAFE_API_KEY environment variable.
api_key = "env:TYPESAFE_API_KEY"

# System One endpoint. Default: "https://api.typesafe.ai/v1/systemone"
endpoint = "https://api.typesafe.ai/v1/systemone"

# Per-request network timeout in milliseconds. Default: 2500 (2.5 seconds).
# If a request exceeds this duration, it immediately aborts and falls back safely.
timeout_ms = 2500

# ---------------------------------------------------------------------------
# UI-Exposed Subsystems (Default to true when enabled = true)
# ---------------------------------------------------------------------------
# Subsystem 1: Classify prompt complexity and dynamically scale reasoning effort
dynamic_thinking = true

# Subsystem 2: Distill clean, error-free terminal and compiler outputs
distill_outputs = true

# Minimum line count required before output distillation is considered. Default: 40.
distill_line_threshold = 40

# Subsystem 3: Auto-approve safe tool calls (read-only bash, file status, inspections)
gate_tools = true

# Maximum risk probability allowed for auto-approval (0.0 to 1.0). Default: 0.20.
# Any command with an estimated risk >= 0.20 prompts the user.
safety_threshold = 0.20

# ---------------------------------------------------------------------------
# Advanced & Experimental Subsystems (Default to false)
# ---------------------------------------------------------------------------
# Subsystem 4: Prune contiguous trailing turns that resulted in a net revert
prune_dead_ends = false

# Subsystem 5: Score and rank competing patch candidates in multi-branch workflows
tournament_pruning = false

# Subsystem 6: Dynamically adjust child subagent thinking effort
dynamic_subagent_thinking = false
```

---

### 5.4 Credential Resolution Order & Security Isolation

To support both shared developer workstations and secure CI/CD runners, credentials resolve in the following strict hierarchy:

1. **Indirect Environment Reference:** If `api_key = "env:VAR_NAME"` is specified in `~/.grok/config.toml`, Grok Build reads that named variable.
2. **Explicit Literal String:** If `api_key = "apikey_..."` is specified directly in `~/.grok/config.toml`.
3. **Standard Environment Variable:** If `api_key` is unset or blank in TOML, Grok Build checks `TYPESAFE_API_KEY`.
4. **Legacy Environment Variable:** If `TYPESAFE_API_KEY` is absent, Grok Build checks `JEV_TYPESAFE_AI_KEY`.
5. **No Credential:** If none of the above are present, Judgment remains completely disabled with zero runtime overhead.

#### Security Invariants:
- **Overlay Protection:** The configuration reader strictly reads the physical file `~/.grok/config.toml`. Malicious repositories cannot use local `.env` files or repository-level `GROK_CONFIG` overrides to hijack your TypeSafe credentials.
- **Log Redaction:** The `JevClient` implements custom `Debug` formatting; API keys are strictly redacted (`api_key = "<redacted>"`) and never written to crash logs or session recordings.

---

### 5.5 How to Verify Judgment is Active

Once enabled, you can verify that Judgment is working in real time during your sessions:

1. **Dynamic Thinking Badge:**
   Look at the reasoning indicator in the status line or prompt box. For simple queries (e.g. *"what does this function return?"*), the model will run at **Low** or **Medium** effort. For deep architectural requests, it automatically scales to **High** or **Extra High**.
2. **Seamless Tool Approvals:**
   When the agent executes read-only inspection commands (such as `ls -la`, `git status`, `git diff`, or `cargo check`), they run immediately without pausing for an interactive approval modal.
3. **Compact Log Previews:**
   Run a command that produces large output (e.g. a 200-line clean test suite). The scrollback will display the first 5 lines, a clean omission pill (e.g., `... 190 lines omitted ...`), and the final status line.
4. **Debug Tracing (Optional):**
   To inspect live Jev scores and response latencies, start Grok Build with debug logging:
   ```bash
   RUST_LOG="info,xai_grok_shell::judgment=debug" grok-build
   ```

---

### 5.6 Troubleshooting & Frequently Asked Questions

#### Q: I turned on Judgment in the settings modal, but commands are still prompting for approval. Why?
> **A:** Session hooks are initialized once when the session is created. If you toggle settings inside an active session, press `Ctrl+C` / exit and restart `grok-build` (or start a new session) for the hook to take effect.

#### Q: What happens if I lose internet connection or my TypeSafe balance runs out?
> **A:** Grok Build guarantees **zero session interruption**. Every Jev call is bounded by a 2.5-second timeout and fails safely:
> - Output distillation fails open (keeps the full log).
> - Safety gating fails closed (prompts you for manual approval).
> - Dynamic reasoning falls back to your configured default thinking effort.
> You will never lose work or experience a crashed session due to an external judgment outage.

#### Q: Can I use Judgment for output distillation and dynamic reasoning, but still manually approve every bash command?
> **A:** Yes! In the Settings modal, keep **TypeSafe Jev judgment** set to `ON`, but set **Judgment safety gating** to `OFF` (or set `gate_tools = false` in `~/.grok/config.toml`). All commands will prompt for your approval while keeping context optimization active.

#### Q: Does TypeSafe Jev send my source code to third-party servers?
> **A:** No. Jev is a non-generative classification model. It only receives bounded micro-payloads (e.g., the command line string being executed, or the compiler output text being filtered) to calculate probability scores. It never receives or writes entire repository codebases.

---

## 6. Summary: Fast, Cost-Effective Pair Programming

The Judgment feature set bridges the gap between frontier reasoning and real-world efficiency. By pairing **xAI Grok's** generative power with **TypeSafe Jev's** instant classification, developers get the best of both worlds:
- **Faster feedback loops** with automatic approval of safe commands.
- **Lower credit burn** through dynamic reasoning and clean context distillation.
- **Extended prompt-cache life** that keeps long-running coding sessions responsive and cost-effective.
