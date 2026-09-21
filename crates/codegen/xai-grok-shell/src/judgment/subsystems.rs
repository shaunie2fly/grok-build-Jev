//! Pure helpers for the token-reduction subsystems.
//!
//! Everything here is deterministic and endpoint-free so it can be unit-tested without a live Jev
//! service. The decision of *whether* to call Jev lives in [`super::hook`]; these functions only
//! shape the text and the thresholds around that decision.

/// Lines retained from the head and the tail when distilling an error-free tool output.
pub const DISTILL_EDGE_LINES: usize = 5;

/// A tool output rewritten into a head/tail excerpt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistilledOutput {
    /// The replacement text that goes into the conversation.
    pub text: String,
    /// How many lines were dropped between the retained head and tail.
    pub omitted_lines: usize,
}

/// Count the lines in `text`.
/// A trailing newline does not start an extra line, so `"a\nb\n"` counts as two.
pub fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let newlines = text.matches('\n').count();
    if text.ends_with('\n') {
        newlines
    } else {
        newlines + 1
    }
}

/// Rewrite an over-threshold, error-free output as its first and last [`DISTILL_EDGE_LINES`] lines
/// plus an explicit omission notice.
///
/// Returns `None` when the output is short enough to keep whole, or when distilling would not
/// actually drop anything (a head and tail that overlap lose no information, so replacing the text
/// would only add noise).
pub fn distill_output(text: &str, line_threshold: usize) -> Option<DistilledOutput> {
    let total = count_lines(text);
    if total <= line_threshold {
        return None;
    }
    let lines: Vec<&str> = text.lines().collect();
    let head = DISTILL_EDGE_LINES.min(lines.len());
    let tail = DISTILL_EDGE_LINES.min(lines.len().saturating_sub(head));
    let omitted = lines.len().saturating_sub(head + tail);
    if omitted == 0 {
        return None;
    }

    let mut out = String::with_capacity(text.len() / 2);
    for line in lines.iter().take(head) {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!(
        "[{omitted} lines omitted; output had no actionable errors. \
         Re-run with a narrower command if you need the middle.]\n"
    ));
    for line in lines.iter().skip(head + omitted) {
        out.push_str(line);
        out.push('\n');
    }
    // Mirror the trailing newline of the input so downstream line counting stays consistent.
    if !text.ends_with('\n') {
        out.pop();
    }
    Some(DistilledOutput {
        text: out,
        omitted_lines: omitted,
    })
}

/// Recognize a shell command that reverts the working tree, which is the only trigger for the
/// contiguous-tail dead-end check.
///
/// Matching is on shell words (`git` `checkout` `--` …) rather than substrings, so a command that
/// merely mentions the words inside an argument does not qualify.
pub fn is_revert_command(command: &str) -> bool {
    // Drop leading `VAR=value` assignments so `FOO=1 git restore .` still matches.
    let tokens: Vec<&str> = command
        .split_whitespace()
        .skip_while(|word| is_env_assignment(word))
        .collect();
    let mut tokens = tokens.as_slice();
    // `sudo` / `command` wrappers do not change what git does.
    if let Some((&first, rest)) = tokens.split_first()
        && (first == "sudo" || first == "command")
    {
        tokens = rest;
    }
    let Some((&program, args)) = tokens.split_first() else {
        return false;
    };
    if program != "git" {
        return false;
    }
    let Some((&subcommand, flags)) = args.split_first() else {
        return false;
    };
    match subcommand {
        // `git checkout -- .`, `git checkout -- <path>`, `git checkout -f`
        "checkout" => flags.contains(&"--") || flags.contains(&"-f"),
        // Every `git restore` form reverts the working tree or the index.
        "restore" => true,
        // `git reset --hard`: a bare `git reset` only unstages, so it does not qualify.
        "reset" => flags.contains(&"--hard"),
        _ => false,
    }
}

/// Whether a shell word is a leading `VAR=value` assignment rather than a command.
fn is_env_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && !name.starts_with('-')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// A planned trim of the conversation's trailing (contiguous) items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TailPrunePlan {
    /// Number of trailing items to remove.
    pub remove_items: usize,
}

/// Plan the contiguous-tail prune for the dead-end scrubber.
///
/// `reverted_items` is how many trailing conversation items belong to the reverted hypothesis.
/// Returns `None` when there is nothing to prune, or when pruning would remove the entire history —
/// a session with no retained prefix has no prompt cache left to preserve, and the spec forbids
/// touching anything outside the contiguous tail.
pub fn plan_tail_prune(history_len: usize, reverted_items: usize) -> Option<TailPrunePlan> {
    if reverted_items == 0 || history_len == 0 {
        return None;
    }
    if reverted_items >= history_len {
        return None;
    }
    Some(TailPrunePlan {
        remove_items: reverted_items,
    })
}

/// Number of trailing prompt turns the dead-end scrubber removes.
pub const DEAD_END_TURNS: usize = 2;

/// Plan the contiguous-tail prune from the conversation's turn boundaries.
///
/// `is_turn_start[i]` marks item `i` as the first item of a prompt turn. The plan keeps everything
/// before the last [`DEAD_END_TURNS`] turns and removes the rest, which is exactly the contiguous
/// trailing region — the prefix the prompt cache is built on is never touched.
///
/// Returns `None` when fewer than [`DEAD_END_TURNS`] + 1 turns exist: pruning would leave no
/// retained prefix, and a summary with no context behind it is worse than the tokens it saves.
pub fn plan_dead_end_prune(is_turn_start: &[bool]) -> Option<TailPrunePlan> {
    let mut starts = is_turn_start
        .iter()
        .enumerate()
        .filter_map(|(index, is_start)| is_start.then_some(index));
    // Keep the first of the last `DEAD_END_TURNS` turns; `None` when there are fewer.
    let keep = starts.nth_back(DEAD_END_TURNS - 1)?;
    // Pruning from the very first turn would leave no prefix for the cache to keep.
    if keep == 0 {
        return None;
    }
    Some(TailPrunePlan {
        remove_items: is_turn_start.len() - keep,
    })
}

/// Confirm a dead-end prune after the Jev round-trip.
///
/// `original_len` / `original_remove` are the plan taken before the verdict. `new_len` /
/// `new_plan` are computed from a fresh conversation read. Returns the item count to remove only
/// when the conversation is the same length and the re-plan matches — otherwise the tail is no
/// longer the one Jev judged, and chopping a stale count would drop the wrong items.
pub fn confirm_dead_end_prune(
    original_len: usize,
    original_remove: usize,
    new_len: usize,
    new_plan: Option<TailPrunePlan>,
) -> Option<usize> {
    if new_len != original_len {
        return None;
    }
    let new_remove = new_plan?.remove_items;
    (new_remove == original_remove).then_some(new_remove)
}

/// The synthetic checkpoint left in place of the pruned turns.
pub fn dead_end_checkpoint_note(pruned_turns: usize) -> String {
    format!(
        "[System: Reverted a failed hypothesis; {pruned_turns} trailing turns pruned to preserve \
         context.]"
    )
}

/// How Subsystem 6 resolved a tool call's approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolGateOutcome {
    /// Jev scored the call non-destructive, so the interactive prompt is skipped.
    JudgmentAllow,
    /// Plan mode already auto-approved the file, so no prompt either way.
    PlanAutoApprove,
    /// Fall through to the interactive permission request.
    AskUser,
}

/// Decide whether the judgment tool gate may skip the interactive approval prompt.
///
/// Precedence, highest first:
/// 1. Plan-mode file auto-approval already resolved the call.
/// 2. A pre-tool-use hook that asked the user keeps its say.
/// 3. `judgment_verdict`: `None` means the gate was not consulted (judgment off, no credential,
///    or a failed call); `Some(true)` is Jev's "non-destructive" verdict.
///
/// Every path except an explicit `Some(true)` asks the user, so a judgment outage can never widen
/// what runs without approval.
pub fn tool_gate_outcome(
    plan_file_auto_approve: bool,
    hook_asked: bool,
    judgment_verdict: Option<bool>,
) -> ToolGateOutcome {
    if plan_file_auto_approve {
        return ToolGateOutcome::PlanAutoApprove;
    }
    if hook_asked {
        return ToolGateOutcome::AskUser;
    }
    match judgment_verdict {
        Some(true) => ToolGateOutcome::JudgmentAllow,
        _ => ToolGateOutcome::AskUser,
    }
}

#[cfg(test)]
#[path = "subsystems_tests.rs"]
mod tests;
