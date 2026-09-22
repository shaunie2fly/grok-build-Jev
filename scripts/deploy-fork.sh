#!/usr/bin/env bash
# Deploy the grok-build-Jev fork as this host's `grok`.
#
# Replaces the npm-installed upstream binary at ~/.grok/bin/grok, which upstream's own
# auto-updater would otherwise silently restore (see the auto_update guard below).
#
# Usage:
#   scripts/deploy-fork.sh                 # build, install, verify
#   scripts/deploy-fork.sh --pull          # git pull --ff-only first, then build
#   scripts/deploy-fork.sh --install-only  # install an already-built artifact
#   scripts/deploy-fork.sh --status        # report current install without changing it
#   scripts/deploy-fork.sh --rollback      # repoint to the previous fork binary
#   scripts/deploy-fork.sh -j 16           # override build parallelism
#
# Env overrides: JOBS, FORK_KEEP (old fork binaries to retain, default 3), GROK_HOME,
# FORK_TAG (version suffix, default "jev"), GROK_VERSION (full version override).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GROK_HOME_DIR="${GROK_HOME:-$HOME/.grok}"
BIN_DIR="$GROK_HOME_DIR/bin"
CONFIG="$GROK_HOME_DIR/config.toml"
STATE="$BIN_DIR/.fork-install-state"
ARTIFACT="$REPO_ROOT/target/release/xai-grok-pager"
KEEP="${FORK_KEEP:-3}"
MANIFEST="$REPO_ROOT/crates/codegen/xai-grok-pager-bin/Cargo.toml"

# Version suffix identifying this fork. `xai-grok-version/build.rs` and the pager-bin
# build.rs both declare `rerun-if-env-changed=GROK_VERSION`, so setting it restamps the
# embedded version. Without it, builds fall back to plain CARGO_PKG_VERSION and are
# indistinguishable from upstream.
# The suffix is also sent as `x-grok-client-version` (proxy version gate) and in the MCP
# user-agent. That gate parses the header as semver and requires >= 0.1.202; a pre-release
# suffix on a 1.0.x base passes, confirmed by direct probe (1.0.38-jev -> 402 billing,
# 0.0.1-garbage -> 426 outdated).
FORK_TAG="${FORK_TAG:-jev}"

JOBS="${JOBS:-}"
DO_BUILD=1
DO_PULL=0
CHECK_JEV=0
ROLLBACK_TO=""
ACTION="deploy"

# Fork markers proving the judgment layer is compiled in. A plain upstream build has none.
FORK_MARKERS=("jev-latest" "api.typesafe.ai" "is_destructive")

log()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m  ok\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m  !! \033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m  FAIL\033[0m %s\n' "$*" >&2; exit 1; }

usage() { awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "${BASH_SOURCE[0]}"; exit 0; }

while [ $# -gt 0 ]; do
  case "$1" in
    --pull)         DO_PULL=1 ;;
    --install-only) DO_BUILD=0 ;;
    --status)       ACTION="status" ;;
    --rollback)     ACTION="rollback" ;;
    --check-jev)    CHECK_JEV=1 ;;
    -j)             JOBS="${2:?--j needs a value}"; shift ;;
    --jobs)         JOBS="${2:?--jobs needs a value}"; shift ;;
    -h|--help)      usage ;;
    *)              die "unknown argument: $1 (try --help)" ;;
  esac
  shift
done

# Read one field from the state file.
# Deliberately does NOT `source` the file: sourcing assigns every key in scope, so a caller
# holding a local named `version`/`commit` would have it silently overwritten by the previous
# install's value.
state_get() {
  [ -f "$STATE" ] || return 0
  sed -nE "s/^$1=(.*)$/\1/p" "$STATE" | head -1
}

read_state() {
  current="$(state_get current)"
  previous="$(state_get previous)"
}

# Atomic symlink swap: build the new link beside the target, then rename over it.
# `ln -sfn` unlinks first, which leaves a window where `grok` does not exist.
atomic_symlink() {
  local target="$1" link="$2" tmp="${2}.tmp.$$"
  ln -s "$target" "$tmp"
  mv -T "$tmp" "$link"
}

# The updater replaces a hand-built fork unless [cli].auto_update is explicitly false.
# Absence is NOT safe: the code path is `auto_update.unwrap_or(true)`, so a missing key
# leaves the updater armed and it will pull the newer npm release over this install.
ensure_auto_update_disabled() {
  [ -f "$CONFIG" ] || { warn "no $CONFIG yet; skipping auto_update guard"; return 0; }
  local result
  result="$(python3 - "$CONFIG" <<'PY'
import re, sys
path = sys.argv[1]
lines = open(path).read().splitlines()
start = next((i for i, l in enumerate(lines) if re.match(r'^\s*\[cli\]\s*$', l)), None)
changed = False
if start is None:
    if lines and lines[-1].strip():
        lines.append('')
    lines += ['[cli]', 'auto_update = false']
    changed = True
else:
    end = next((j for j in range(start + 1, len(lines)) if re.match(r'^\s*\[', lines[j])), len(lines))
    key = next((k for k in range(start + 1, end) if re.match(r'^\s*auto_update\s*=', lines[k])), None)
    if key is None:
        lines.insert(start + 1, 'auto_update = false')
        changed = True
    elif lines[key].strip() != 'auto_update = false':
        lines[key] = 'auto_update = false'
        changed = True
if changed:
    open(path, 'w').write('\n'.join(lines) + '\n')
print('changed' if changed else 'ok')
PY
)"
  case "$result" in
    changed)
      warn "auto_update was armed; set [cli].auto_update = false (backup: ${CONFIG}.bak)"
      cp -n "$CONFIG" "${CONFIG}.bak" 2>/dev/null || true
      ;;
    ok) ok "auto_update already disabled" ;;
    *)  warn "auto_update guard returned unexpected result: $result" ;;
  esac
  if grep -qE '^installer\s*=\s*"(internal|gh-release)"' "$CONFIG" 2>/dev/null; then
    warn 'installed installer = "internal"/"gh-release": its heal step also rewrites'
    warn '~/.grok/bin/{grok,agent}, so it would clobber this install even with auto_update off.'
  fi
}

# Parallelism default. Empirically proven on this host: `-j8` completes the 1450-unit
# release build in ~40 min cold. The guard exists only because the box has 14 GB RAM and a
# desktop session that can leave MemAvailable near 2 GB; below that, linking thrashes and
# can OOM. Override with JOBS= or -j.
resolve_jobs() {
  [ -n "$JOBS" ] && { echo "$JOBS"; return; }
  local cores avail_gb
  cores="$(nproc 2>/dev/null || echo 4)"
  avail_gb="$(awk '/MemAvailable/{printf "%d", $2/1048576}' /proc/meminfo 2>/dev/null || echo 8)"
  local jobs=8
  if   [ "$avail_gb" -lt 2 ]; then jobs=2
  elif [ "$avail_gb" -lt 3 ]; then jobs=4
  fi
  [ "$jobs" -gt "$cores" ] && jobs="$cores"
  echo "$jobs"
}

build() {
  command -v cargo >/dev/null || die "cargo not found on PATH"
  local jobs; jobs="$(resolve_jobs)"

  # Stamp the fork tag into the embedded version. Both build scripts that consume
  # GROK_VERSION declare `rerun-if-env-changed`, so passing it here restamps
  # xai-grok-version::VERSION and pager-bin's VERSION_WITH_COMMIT.
  local base
  base="$(sed -nE 's/^version = "([^"]+)".*/\1/p' "$MANIFEST" | head -1)"
  [ -n "$base" ] || die "no version found in $MANIFEST"
  export GROK_VERSION="${GROK_VERSION:-${base}-${FORK_TAG}}"
  log "Stamping version $GROK_VERSION"

  log "Building fork release (jobs=$jobs, ~40 min cold, minutes warm)"
  if [ "$DO_PULL" -eq 1 ]; then
    git -C "$REPO_ROOT" diff --quiet || die "--pull refused: working tree has uncommitted changes"
    log "git pull --ff-only"
    git -C "$REPO_ROOT" pull --ff-only
  fi
  ( cd "$REPO_ROOT" && cargo build -p xai-grok-pager-bin --release -j "$jobs" ) \
    || die "cargo build failed"
  ok "build complete"
}

# Anchor identity to the artifact itself: the binary embeds its own version + commit.
probe_artifact() {
  [ -x "$ARTIFACT" ] || die "artifact missing: $ARTIFACT (run without --install-only)"
  local out
  out="$(timeout 180 "$ARTIFACT" --version 2>&1)" || die "--version failed on the built artifact"
  # e.g. "grok 1.0.38 (050d560bc6f8)"
  local version sha
  version="$(printf '%s' "$out" | sed -nE 's/^grok ([0-9][^ ]*).*/\1/p')"
  sha="$(printf '%s' "$out" | sed -nE 's/.*\(([0-9a-f]{7,})\).*/\1/p')"
  [ -n "$version" ] || die "could not parse version from: $out"
  printf '%s %s\n' "$version" "${sha:0:7}"
}

# Content checks on a single binary.
# `require_tag` (1/0): a FRESH build must carry the fork tag; a rollback target legitimately
# may not (it predates the tag), so there the tag is only reported, never fatal.
# `failed` accumulates across calls.
verify_binary() {
  local path="$1" label="$2" require_tag="${3:-1}" failed=0

  # Static link check: a build against a missing lib would otherwise fail at first launch.
  # Capture ldd output first; `ldd | grep -q` is unsafe under `set -o pipefail` (SIGPIPE).
  local ldd_out
  ldd_out="$(ldd "$path" 2>/dev/null || true)"
  if printf '%s\n' "$ldd_out" | grep -q 'not found'; then
    warn "$label: missing shared libraries:"; printf '%s\n' "$ldd_out" | grep 'not found' | sed 's/^/     /' >&2; failed=1
  else
    ok "$label: shared libraries resolve"
  fi

  # Fork identity: these strings only exist when the judgment layer is compiled in.
  # Dump strings ONCE (a 230 MB binary costs ~13 s per call) inside a subshell whose EXIT
  # trap cleans the temp file. A `RETURN` trap here would leak out of this function and re-fire
  # in the caller, where the local is unset and `set -u` aborts the script.
  # `strings | grep -q` is not used: under `set -o pipefail`, grep -q closing the pipe early
  # makes strings die with SIGPIPE (141), failing the pipeline even on a hit.
  local missing
  missing="$(
    d="$(mktemp)"; trap 'rm -f "$d"' EXIT
    strings -a "$path" > "$d" 2>/dev/null || true
    for m in "${FORK_MARKERS[@]}"; do
      grep -qF -- "$m" "$d" || printf '%s ' "$m"
    done
  )"
  if [ -z "$missing" ]; then
    ok "$label: fork markers present"
  else
    warn "$label: fork markers ABSENT: $missing — not a fork build"; failed=1
  fi

  # The fork tag is only present when GROK_VERSION was set at build time. Its absence means the
  # binary is indistinguishable from an unstamped upstream build, so fail loudly rather than
  # install something that silently claims to be stock grok.
  local v
  if v="$(timeout 180 "$path" --version 2>&1)"; then
    ok "$label: reports $v"
    if printf '%s' "$v" | grep -q -- "-${FORK_TAG}"; then
      ok "$label: version carries '-${FORK_TAG}'"
    elif [ "$require_tag" -eq 1 ]; then
      warn "$label: version '$v' lacks '-${FORK_TAG}' (built without GROK_VERSION=<ver>-${FORK_TAG})"; failed=1
    else
      warn "$label: version '$v' predates the '-${FORK_TAG}' tag (expected for a rollback target)"
    fi
  else
    warn "$label: --version failed"; failed=1
  fi

  return "$failed"
}

# Installation-state checks: the symlinks and PATH resolution that `verify_binary` cannot see.
verify() {
  local name="$1" require_tag="${2:-1}" failed=0

  log "Verifying $name"
  [ -x "$BIN_DIR/$name" ] || { warn "not executable: $BIN_DIR/$name"; failed=1; }

  verify_binary "$BIN_DIR/$name" "$name" "$require_tag" || failed=1

  # Both entrypoints matter: `agent` drives stdio/ACP/editor integrations and does NOT
  # follow the `grok` symlink.
  for entry in grok agent; do
    local tgt; tgt="$(readlink "$BIN_DIR/$entry" 2>/dev/null || echo '')"
    if [ "$tgt" = "$name" ]; then ok "$entry -> $name"
    else warn "$entry points at '${tgt:-<not a symlink>}', expected '$name'"; failed=1; fi
  done

  local resolved; resolved="$(command -v grok 2>/dev/null || echo '')"
  if [ -z "$resolved" ]; then warn "grok not on PATH"; failed=1
  else ok "PATH resolves grok -> $(readlink -f "$resolved")"; fi

  [ "$failed" -eq 0 ] || die "verification failed; see warnings above"
}

check_jev() {
  local key
  key="$(sed -nE 's/^[[:space:]]*api_key[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$CONFIG" 2>/dev/null | head -1)"
  if [ -z "$key" ]; then
    key="${TYPESAFE_API_KEY:-${JEV_TYPESAFE_AI_KEY:-}}"
  fi
  [ -n "$key" ] || { warn "--check-jev: no TypeSafe key in $CONFIG or environment; skipping"; return 0; }
  log "Probing the Jev judgment endpoint"
  local code
  code="$(curl -s -o /dev/null -w '%{http_code}' -X POST https://api.typesafe.ai/v1/systemone \
    -H "Authorization: Bearer $key" -H 'Content-Type: application/json' \
    -d '{"model":"jev-latest","state":{"prompt":"probe"},"questions":{"complexity":{"type":"score","instructions":"Evaluate difficulty.","criteria":["a","b","c","d"]}}}' \
    2>/dev/null)" || code=000
  [ -n "$code" ] || code=000
  case "$code" in
    200) ok "Jev endpoint reachable (HTTP 200)" ;;
    401|403) warn "Jev rejected the credential (HTTP $code)" ;;
    *)   warn "Jev probe returned HTTP $code (not a build defect)"; ;;
  esac
}

install() {
  local version="$1" sha="$2"
  # The version already carries the fork tag (e.g. 1.0.38-jev), so no `-fork-` marker.
  local name="grok-${version}-${sha}"
  local prev; read_state; prev="${current:-}"

  # Gate BEFORE touching anything: an untagged or broken artifact must never reach the
  # symlink swap, or the host ends up running it while the script reports failure.
  log "Pre-flight checks on the staged artifact"
  verify_binary "$ARTIFACT" "artifact" \
    || die "artifact failed pre-flight checks; nothing installed (host still on ${prev:-current})"

  mkdir -p "$BIN_DIR"
  if [ -f "$BIN_DIR/$name" ] && cmp -s "$ARTIFACT" "$BIN_DIR/$name"; then
    log "Installing $name (identical binary already present)"
  else
    log "Installing $name"
    # Copy beside the destination so the rename cannot cross filesystems.
    cp "$ARTIFACT" "$BIN_DIR/.$name.tmp.$$"
    chmod 0755 "$BIN_DIR/.$name.tmp.$$"
    mv -f "$BIN_DIR/.$name.tmp.$$" "$BIN_DIR/$name"
  fi
  ok "installed $BIN_DIR/$name"

  atomic_symlink "$name" "$BIN_DIR/grok"
  atomic_symlink "$name" "$BIN_DIR/agent"
  ok "entrypoints grok, agent -> $name"

  ensure_auto_update_disabled

  # Retain the previous fork so --rollback has a target.
  local previous="$prev"
  [ "$previous" = "$name" ] && previous=""
  if [ -n "$previous" ] && [ ! -x "$BIN_DIR/$previous" ]; then
    warn "previous install '$previous' no longer exists; rollback cleared"
    previous=""
  fi

  {
    printf 'current=%s\n'  "$name"
    printf 'previous=%s\n' "$previous"
    printf 'version=%s\n'  "$version"
    printf 'commit=%s\n'   "$sha"
    printf 'built_at=%s\n' "$(date -Iseconds)"
  } > "$STATE.tmp.$$" && mv -f "$STATE.tmp.$$" "$STATE"

  prune

  if [ "$CHECK_JEV" -eq 1 ]; then check_jev; fi
  verify "$name"
  log "Done. Running: $name"
  [ -n "$previous" ] && log "Rollback target: $previous  (scripts/deploy-fork.sh --rollback)"
  return 0
}

# Old fork binaries are ~230 MB each and live on the root filesystem.
prune() {
  local all
  # Match both naming generations: legacy `grok-1.0.38-fork-050d560` and the current
  # `grok-1.0.38-jev-050d560` (fork tag now lives in the version string).
  mapfile -t all < <(find "$BIN_DIR" -maxdepth 1 \( -name 'grok-*-fork*' -o -name "grok-*-${FORK_TAG}-*" \) -type f -printf '%f\n' 2>/dev/null | sort -uV)
  local n=${#all[@]}
  [ "$n" -le "$KEEP" ] && return 0
  local read_state_current; read_state; read_state_current="${current:-}"
  local idx=0
  for f in "${all[@]}"; do
    if [ $((n - idx)) -gt "$KEEP" ] && [ "$f" != "$read_state_current" ]; then
      rm -f "$BIN_DIR/$f" && ok "pruned old fork binary: $f"
    fi
    idx=$((idx + 1))
  done
}

status() {
  log "Fork install status"
  local cur prev ver com bui
  cur="$(state_get current)"; prev="$(state_get previous)"
  ver="$(state_get version)"; com="$(state_get commit)"; bui="$(state_get built_at)"
  printf '  current : %s\n' "${cur:-<none>}"
  printf '  previous: %s\n' "${prev:-<none>}"
  printf '  version : %s\n' "${ver:-<unknown>}"
  printf '  commit  : %s\n' "${com:-<unknown>}"
  printf '  built_at: %s\n' "${bui:-<unknown>}"
  printf '  home    : %s\n' "$GROK_HOME_DIR"
  local tgt; tgt="$(readlink "$BIN_DIR/grok" 2>/dev/null || echo '<none>')"
  printf '  grok -> : %s\n' "$tgt"
  printf '  agent-> : %s\n' "$(readlink "$BIN_DIR/agent" 2>/dev/null || echo '<none>')"
  printf '  PATH    : %s\n' "$(command -v grok 2>/dev/null || echo '<not found>')"
  printf '  auto_upd: %s\n' "$(sed -nE 's/^[[:space:]]*auto_update[[:space:]]*=[[:space:]]*(.*)$/\1/p' "$CONFIG" 2>/dev/null | head -1 || echo '<unset>')"
  printf '  repo    : %s\n' "$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo '<not a repo>')"
  echo
  log "Installed fork binaries (${KEEP} newest retained)"
  find "$BIN_DIR" -maxdepth 1 \( -name 'grok-*-fork*' -o -name "grok-*-${FORK_TAG}-*" \) -type f -printf '  %f  %s bytes  %TY-%Tm-%Td %TH:%TM\n' 2>/dev/null | sort -uV
}

rollback() {
  read_state
  local target="${ROLLBACK_TO:-${previous:-}}"
  [ -n "$target" ] || die "no rollback target recorded; install twice before rolling back"
  [ -x "$BIN_DIR/$target" ] || die "rollback target missing: $BIN_DIR/$target"
  local current_before="${current:-}"
  log "Rolling back to $target"
  # A rollback target may predate the fork tag, so the tag is reported but not required.
  verify_binary "$BIN_DIR/$target" "$target" 0 || die "rollback target failed checks; symlinks untouched"
  atomic_symlink "$target" "$BIN_DIR/grok"
  atomic_symlink "$target" "$BIN_DIR/agent"
  {
    printf 'current=%s\n'  "$target"
    printf 'previous=%s\n' "$current_before"
    # Report the target's real version rather than guessing from the filename, which carries
    # a commit suffix (and on legacy names, a `-fork` marker).
    printf 'version=%s\n'  "$(timeout 180 "$BIN_DIR/$target" --version 2>/dev/null | sed -nE 's/^grok ([^ ]+).*/\1/p' || true)"
    printf 'commit=%s\n'   ""
    printf 'built_at=%s\n' "$(date -Iseconds)"
  } > "$STATE.tmp.$$" && mv -f "$STATE.tmp.$$" "$STATE"
  verify "$target" 0
  ok "rolled back to $target"
}

main() {
  case "$ACTION" in
    status)   status ;;
    rollback) rollback ;;
    deploy)
      [ -d "$REPO_ROOT" ] || die "repo root not found: $REPO_ROOT"
      cd "$REPO_ROOT"
      if [ "$DO_BUILD" -eq 1 ]; then build; else log "Skipping build (--install-only)"; fi
      local version sha
      read -r version sha <<<"$(probe_artifact)"
      ok "artifact: grok $version ($sha)"
      install "$version" "$sha"
      ;;
  esac
}

main
