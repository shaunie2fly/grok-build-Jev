//! Public surface of the Jev judgment integration.

pub mod client;
pub mod evaluator;
pub mod hook;
pub mod subsystems;
pub mod tournament;

pub use client::{JevClient, JevError};
pub use evaluator::JudgmentEvaluator;
pub use hook::JudgmentHook;
pub use subsystems::{
    DEAD_END_TURNS, DISTILL_EDGE_LINES, DistilledOutput, TailPrunePlan, ToolGateOutcome,
    confirm_dead_end_prune, count_lines, dead_end_checkpoint_note, distill_output,
    is_revert_command, plan_dead_end_prune, plan_tail_prune, tool_gate_outcome,
};
pub use tournament::{
    CandidateScore, TournamentRanking, rank_candidates, tournament_resolution_line,
};
