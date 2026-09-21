<div align="center">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://media.x.ai/v1/website/spacexai-symbol-white-transparent-0c31957f.png">
    <source media="(prefers-color-scheme: light)" srcset="https://media.x.ai/v1/website/spacexai-symbol-black-transparent-6435cf42.png">
    <img alt="SpaceXAI logo" src="https://media.x.ai/v1/website/spacexai-symbol-black-transparent-6435cf42.png" width="96">
  </picture>
  <br>
  Grok Build (<code>grok</code>)
</h1>

**Grok Build** is SpaceXAI's terminal-based AI coding agent. It runs as a
full-screen TUI that understands your codebase, edits files, executes shell
commands, searches the web, and manages long-running tasks — interactively,
headlessly for scripting/CI, or embedded in editors via the Agent Client
Protocol (ACP).

This fork integrates **TypeSafe Jev** as a fast System 1 micro-decision layer,
providing intelligent token optimization, dynamic reasoning allocation, and
silent tool safety gating.

[TypeSafe Jev Judgment](#typesafe-jev-judgment-system-1) ·
[Installing the released binary](#installing-the-released-binary) ·
[Building from source](#building-from-source) ·
[Documentation](#documentation) ·
[Repository layout](#repository-layout) ·
[Development](#development) ·
[Contributing](#contributing) ·
[License](#license)

![Grok Build TUI](https://media.x.ai/v1/website/universe-tui-screenshot-6f7a0837.png)

**Learn more about Grok Build at [x.ai/cli](https://x.ai/cli) and TypeSafe Jev at [typesafe.ai](https://typesafe.ai)**

This repository contains the Rust source for the `grok` CLI/TUI and its agent
runtime. It is synced periodically from the SpaceXAI monorepo.

A small `SOURCE_REV` file at the root records the full monorepo commit SHA
for the version of the code present in this tree.

</div>

---

## TypeSafe Jev Judgment (System 1)

This repository fork integrates **[TypeSafe Jev](https://typesafe.ai)** (`jev-latest` via `/v1/systemone`) as a fast, non-generative **System 1** micro-decision layer paired with frontier models like **Grok 4.6**.

Autonomous coding agents frequently burn excessive tokens and stall developer momentum on micro-decisions that do not require frontier intelligence. By delegating these decisions to Jev, Grok Build preserves reasoning tokens and prompt-cache lifetime for actual code generation.

### The 5 Subsystems

1. **Dynamic Reasoning Effort Allocation:** Classifies prompt complexity in ~200ms and dynamically sets thinking effort (`low`, `medium`, `high`, `xhigh`) per turn, saving tens of thousands of unnecessary thinking tokens on simple edits.
2. **Context Output Distillation:** Automatically compresses clean, repetitive 200–500+ line compiler and test logs (`cargo test`, `pytest`) into concise excerpts while preserving full stack traces whenever actionable errors occur.
3. **Non-Destructive Tool Safety Gating:** Silently auto-approves safe read-only inspection commands (`ls`, `git diff`, `git status`, `cargo check`), eliminating approval fatigue while keeping destructive commands strictly gated for manual confirmation.
4. **Dead-End Hypothesis Pruning:** Detects reverted debugging spikes at the conversation tail and prunes them with a synthetic checkpoint without invalidating provider prefix prompt-caching.
5. **Tournament Patch Quality Scoring:** Evaluates and scores competing git diffs against task requirements in speculative multi-branch workflows.

### Real-World Benefits

- **60%–70% cost reduction** across standard 20-turn sessions.
- **Zero approval friction** on read-only exploration and inspection steps.
- **Sub-300ms latency overhead** with fail-safe fallbacks (fail-open for distillation, fail-closed for safety).

### Quick Setup

1. Export your API key in your environment or add it to `~/.grok/config.toml`:
   ```bash
   export TYPESAFE_API_KEY="apikey_your_actual_key_here"
   ```
2. Enable Judgment via the interactive Settings Modal (**`F2`** or **`Ctrl+,`** $\rightarrow$ **Agent** $\rightarrow$ **TypeSafe Jev judgment**), or configure `~/.grok/config.toml`:
   ```toml
   [judgment]
   enabled = true
   api_key = "env:TYPESAFE_API_KEY"
   dynamic_thinking = true
   distill_outputs = true
   gate_tools = true
   ```
3. Restart `grok-build` to initialize the active hook.

📖 **For detailed architecture, economics, and configuration instructions, see the [Full Judgment Feature Guide](docs/judgment_feature_guide.md).**

---

## Installing the released binary

Prebuilt binaries are published for macOS, Linux, and Windows:

```sh
curl -fsSL https://x.ai/cli/install.sh | bash   # macOS / Linux / Git Bash
irm https://x.ai/cli/install.ps1 | iex          # Windows PowerShell
grok --version
```

See the [changelog](https://x.ai/build/changelog) for the latest fixes,
features, and improvements in each release.

## Building from source

Requirements:

- **Rust** — the toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml);
  `rustup` installs it automatically on first build.
- **[DotSlash](https://dotslash-cli.com)** — required so hermetic tools under
  [`bin/`](bin/) (notably [`bin/protoc`](bin/protoc)) can download and run.
  Install it and ensure `dotslash` is on your `PATH` **before** building:

  ```sh
  cargo install dotslash
  # or: prebuilt packages — https://dotslash-cli.com/docs/installation/
  /usr/bin/env dotslash --help   # sanity check
  ```

- **protoc** — proto codegen resolves [`bin/protoc`](bin/protoc) via DotSlash,
  or falls back to a `protoc` on `PATH` / `$PROTOC`.
- macOS and Linux are supported build hosts; Windows builds are best-effort
  and not currently tested from this tree.

```sh
cargo run -p xai-grok-pager-bin              # build + launch the TUI
cargo build -p xai-grok-pager-bin --release  # release binary: target/release/xai-grok-pager
cargo check -p xai-grok-pager-bin            # fast validation
```

The binary artifact is named `xai-grok-pager`; official installs ship it as
`grok`. On first launch it opens your browser to authenticate — see the
[authentication guide](crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md).

## Documentation

Full online documentation is available at
[docs.x.ai/build/overview](https://docs.x.ai/build/overview).

The user guide ships with the pager crate:
[`crates/codegen/xai-grok-pager/docs/user-guide/`](crates/codegen/xai-grok-pager/docs/user-guide/)
— getting started, keyboard shortcuts, slash commands, configuration, theming,
MCP servers, skills, plugins, hooks, headless mode, sandboxing, and more.

## Repository layout

| Path | Contents |
|------|----------|
| `crates/codegen/xai-grok-pager-bin` | Composition-root package; builds the `xai-grok-pager` binary |
| `crates/codegen/xai-grok-pager` | The TUI: scrollback, prompt, modals, rendering |
| `crates/codegen/xai-grok-shell` | Agent runtime + leader/stdio/headless entry points |
| `crates/codegen/xai-grok-tools` | Tool implementations (terminal, file edit, search, ...) |
| `crates/codegen/xai-grok-workspace` | Host filesystem, VCS, execution, checkpoints |
| `crates/codegen/...` | The rest of the CLI crate closure (config, MCP, markdown, sandbox, ...) |
| `crates/common/`, `crates/build/`, `prod/mc/` | Small shared leaf crates pulled in by the closure |
| `third_party/` | Vendored upstream source (Mermaid diagram stack) — see below |

> [!IMPORTANT]
> The root `Cargo.toml` (workspace members, dependency versions, lints,
> profiles) is **generated** — treat it as read-only. Prefer editing per-crate
> `Cargo.toml` files.

## Development

```sh
cargo check -p <crate>        # always target specific crates; full-workspace builds are slow
cargo test -p xai-grok-config # per-crate tests
cargo clippy -p <crate>       # lint config: clippy.toml at the repo root
cargo fmt --all               # rustfmt.toml at the repo root
```

## Contributing

> [!NOTE]
> External contributions are not accepted. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

First-party code in this repository is licensed under the **Apache License,
Version 2.0** — see [`LICENSE`](LICENSE).

Third-party and vendored code remains under its original licenses. See:

- [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES) — crates.io / git dependencies,
  bundled UI themes, and **in-tree source ports** (including openai/codex and
  sst/opencode tool implementations)
- [`crates/codegen/xai-grok-tools/THIRD_PARTY_NOTICES.md`](crates/codegen/xai-grok-tools/THIRD_PARTY_NOTICES.md)
  — crate-local notice for the codex and opencode ports (license texts +
  Apache §4(b) change notice)
- [`third_party/NOTICE`](third_party/NOTICE) — vendored Mermaid-stack index
