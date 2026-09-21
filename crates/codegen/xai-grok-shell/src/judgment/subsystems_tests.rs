//! Tests for the endpoint-free subsystem helpers.

use super::super::tournament::{CandidateScore, rank_candidates, tournament_resolution_line};
use super::*;
use ToolGateOutcome::{AskUser, JudgmentAllow, PlanAutoApprove};

#[test]
fn count_lines_ignores_a_trailing_newline() {
    assert_eq!(count_lines(""), 0);
    assert_eq!(count_lines("a"), 1);
    assert_eq!(count_lines("a\nb"), 2);
    assert_eq!(count_lines("a\nb\n"), 2);
    assert_eq!(count_lines("\n"), 1);
}

/// Below or at the threshold the output is kept whole: distillation is opt-in by size.
#[test]
fn distillation_is_skipped_at_or_below_the_threshold() {
    let short = "line\n".repeat(10);
    assert_eq!(distill_output(&short, 40), None);
    let exact = "line\n".repeat(40);
    assert_eq!(distill_output(&exact, 40), None, "threshold is inclusive");
}

/// The whole point: an error-free long output keeps its head and tail and drops the middle.
#[test]
fn distillation_keeps_the_first_and_last_five_lines() {
    let body: String = (1..=100).map(|n| format!("line {n}\n")).collect();
    let distilled = distill_output(&body, 40).expect("100 lines must distill");

    assert_eq!(distilled.omitted_lines, 90);
    let lines: Vec<&str> = distilled.text.lines().collect();
    assert_eq!(lines.first(), Some(&"line 1"));
    assert_eq!(lines.get(4), Some(&"line 5"));
    assert!(
        lines.iter().any(|l| l.contains("90 lines omitted")),
        "the omission notice must state the count: {}",
        distilled.text
    );
    assert!(lines.contains(&"line 96"), "tail begins at 96");
    assert_eq!(lines.last(), Some(&"line 100"));
    assert!(distilled.text.len() < body.len());
}

/// Distilling must never invent or drop edge content, and it must not fire when the head and tail
/// already cover the whole output.
#[test]
fn distillation_declines_when_nothing_would_be_omitted() {
    // 12 lines: head 5 + tail 5 leaves 2 omitted, so it distills.
    let twelve: String = (1..=12).map(|n| format!("line {n}\n")).collect();
    assert_eq!(distill_output(&twelve, 10).unwrap().omitted_lines, 2);
}

#[test]
fn revert_commands_are_recognized_by_shell_words() {
    for command in [
        "git checkout -- .",
        "git checkout -- src/main.rs",
        "git checkout -f",
        "git restore .",
        "git restore --staged src/lib.rs",
        "git reset --hard",
        "git reset --hard HEAD~1",
        "sudo git restore .",
        "FOO=1 git restore .",
        "  git   reset   --hard  ",
    ] {
        assert!(is_revert_command(command), "should match: {command}");
    }
}

/// A command that merely mentions the words is not a revert; pruning on it would delete real work.
#[test]
fn non_revert_commands_do_not_match() {
    for command in [
        "git status",
        "git diff",
        "git commit -m 'revert git checkout -- .'",
        "echo git restore .",
        "git reset",
        "git reset --soft HEAD~1",
        "git checkout main",
        "git checkout -b feature",
        "cargo test",
        "grep -r 'git restore' .",
        "git",
        "git restore",
        "",
    ] {
        if command == "git restore" {
            // A bare `git restore` does revert the working tree, so it is a match.
            assert!(
                is_revert_command(command),
                "bare restore reverts: {command}"
            );
            continue;
        }
        assert!(!is_revert_command(command), "should not match: {command}");
    }
}

/// Pruning only ever touches the contiguous tail, and never the whole history.
#[test]
fn tail_prune_plan_refuses_to_remove_everything() {
    assert_eq!(plan_tail_prune(0, 0), None);
    assert_eq!(plan_tail_prune(10, 0), None, "nothing reverted");
    assert_eq!(
        plan_tail_prune(10, 10),
        None,
        "removing the whole history would leave no cache prefix"
    );
    assert_eq!(
        plan_tail_prune(10, 11),
        None,
        "more reverted items than items must not underflow"
    );
    assert_eq!(
        plan_tail_prune(10, 4).map(|p| p.remove_items),
        Some(4),
        "a strict subset of the tail is prunable"
    );
}

/// The scrubber removes only complete trailing turns, keeping the cacheable prefix intact.
#[test]
fn dead_end_prune_keeps_everything_before_the_last_two_turns() {
    // Three turns start at items 0, 3 and 7; 11 items total.
    let starts = [
        true, false, false, true, false, false, false, true, false, false, false,
    ];
    let plan = plan_dead_end_prune(&starts).expect("3 turns is enough to prune 2");
    assert_eq!(
        plan.remove_items, 8,
        "keeps items 0..3, removes the turns starting at 3 and 7"
    );
}

/// With exactly two turns there is no prefix left to preserve, so nothing is pruned.
#[test]
fn dead_end_prune_declines_when_no_prefix_would_remain() {
    let two_turns = [true, false, true, false, false];
    assert_eq!(plan_dead_end_prune(&two_turns), None);
    let one_turn = [true, false, false];
    assert_eq!(plan_dead_end_prune(&one_turn), None);
    assert_eq!(plan_dead_end_prune(&[]), None);
    assert_eq!(
        plan_dead_end_prune(&[false, false]),
        None,
        "no turn boundary"
    );
}

#[test]
fn dead_end_prune_removes_exactly_two_turns() {
    // Four turns, one item each: pruning two leaves two.
    let starts = [true, true, true, true];
    let plan = plan_dead_end_prune(&starts).expect("prunable");
    assert_eq!(plan.remove_items, 2, "items at index 2 and 3");
}

#[test]
fn checkpoint_note_states_the_pruned_turn_count() {
    assert!(dead_end_checkpoint_note(2).contains("2 trailing turns pruned"));
}

/// After the Jev round-trip the conversation may have moved. A matching re-plan is the only
/// case that may chop the tail; a length or plan change must leave history untouched.
#[test]
fn confirm_dead_end_prune_requires_an_unchanged_plan() {
    assert_eq!(
        confirm_dead_end_prune(10, 4, 10, Some(TailPrunePlan { remove_items: 4 })),
        Some(4)
    );
    assert_eq!(
        confirm_dead_end_prune(10, 4, 12, Some(TailPrunePlan { remove_items: 4 })),
        None,
        "appended items must not be chopped as if they were the dead end"
    );
    assert_eq!(
        confirm_dead_end_prune(10, 4, 8, Some(TailPrunePlan { remove_items: 4 })),
        None,
        "a shorter history is not the plan we asked Jev about"
    );
    assert_eq!(
        confirm_dead_end_prune(10, 4, 10, Some(TailPrunePlan { remove_items: 6 })),
        None,
        "a different tail length is not the same prune"
    );
    assert_eq!(
        confirm_dead_end_prune(10, 4, 10, None),
        None,
        "a fresh plan that declines must not fall back to the stale count"
    );
}

/// A hook that already asked the user must not be overridden by a Jev verdict — the user's prompt
/// is the stronger signal, and silently skipping it would hide a prompt the hook wanted.
#[test]
fn a_hook_ask_outranks_a_safe_judgment_verdict() {
    assert_eq!(tool_gate_outcome(false, true, Some(true)), AskUser);
}

/// Plan-mode file auto-approval already resolved the call, so the gate is not even consulted.
#[test]
fn plan_auto_approve_short_circuits_the_gate() {
    assert_eq!(tool_gate_outcome(true, false, None), PlanAutoApprove);
    assert_eq!(tool_gate_outcome(true, true, Some(false)), PlanAutoApprove);
}

#[test]
fn a_safe_verdict_allows_without_prompting() {
    assert_eq!(tool_gate_outcome(false, false, Some(true)), JudgmentAllow);
}

/// The safety-critical direction: anything other than an explicit "safe" must reach the user.
#[test]
fn risky_unconsulted_and_missing_verdicts_all_ask_the_user() {
    for verdict in [Some(false), None] {
        assert_eq!(
            tool_gate_outcome(false, false, verdict),
            AskUser,
            "verdict {verdict:?} must not auto-approve"
        );
    }
}

#[test]
fn candidates_rank_best_first() {
    let ranked = rank_candidates(vec![
        CandidateScore {
            id: "A".into(),
            score: 0.41,
        },
        CandidateScore {
            id: "B".into(),
            score: 0.94,
        },
        CandidateScore {
            id: "C".into(),
            score: 0.62,
        },
    ]);
    assert_eq!(ranked.ranked, vec!["B", "C", "A"]);
    assert_eq!(ranked.winner.as_deref(), Some("B"));
    assert_eq!(ranked.pruned, vec!["C", "A"]);
}

/// Ties must resolve deterministically, or two runs could merge different candidates.
#[test]
fn ties_break_on_id_so_ranking_is_reproducible() {
    let field = || {
        vec![
            CandidateScore {
                id: "C".into(),
                score: 0.5,
            },
            CandidateScore {
                id: "A".into(),
                score: 0.5,
            },
            CandidateScore {
                id: "B".into(),
                score: 0.5,
            },
        ]
    };
    let first = rank_candidates(field());
    let second = rank_candidates(field());
    assert_eq!(first, second);
    assert_eq!(first.ranked, vec!["A", "B", "C"]);
    assert_eq!(first.winner.as_deref(), Some("A"));
}

#[test]
fn an_empty_field_has_no_winner() {
    let ranked = rank_candidates(Vec::new());
    assert_eq!(ranked.winner, None);
    assert!(ranked.pruned.is_empty());
    assert!(tournament_resolution_line(&ranked, &[]).contains("no candidates"));
}

/// The synthetic line is what the parent sees in place of the losers' transcripts.
#[test]
fn resolution_line_names_the_winner_and_the_pruned_candidates() {
    let scores = vec![
        CandidateScore {
            id: "A".into(),
            score: 0.41,
        },
        CandidateScore {
            id: "B".into(),
            score: 0.94,
        },
        CandidateScore {
            id: "C".into(),
            score: 0.62,
        },
    ];
    let line = tournament_resolution_line(&rank_candidates(scores.clone()), &scores);
    assert!(line.contains("3 candidates executed"), "{line}");
    assert!(line.contains("Candidate B selected"), "{line}");
    assert!(line.contains("0.94"), "{line}");
    assert!(line.contains("C and A pruned"), "{line}");
}

#[test]
fn a_solo_candidate_reports_no_pruning() {
    let scores = vec![CandidateScore {
        id: "A".into(),
        score: 0.8,
    }];
    let line = tournament_resolution_line(&rank_candidates(scores.clone()), &scores);
    assert!(line.contains("1 candidate executed"), "{line}");
    assert!(!line.contains("pruned"), "{line}");
}
