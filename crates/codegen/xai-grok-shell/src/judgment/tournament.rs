//! Tournament candidate ranking (A2: scoring primitive).
//!
//! This repository has no parallel-candidate tournament executor, so nothing here drives a merge.
//! What ships is the ranking decision itself: given scored candidates, which one wins and which
//! are pruned. When a tournament executor exists, it calls [`rank_candidates`] and acts on the
//! result; until then the function is unit-tested and ready.

/// One scored tournament candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateScore {
    /// Stable candidate identifier (for example `A` / `B` / `C`).
    pub id: String,
    /// Jev quality score in `0.0..=1.0`; higher is better.
    pub score: f64,
}

/// The outcome of ranking a tournament field.
#[derive(Debug, Clone, PartialEq)]
pub struct TournamentRanking {
    /// Candidate ids ordered best-first.
    pub ranked: Vec<String>,
    /// The winning candidate id, or `None` when the field was empty.
    pub winner: Option<String>,
    /// Candidate ids to discard, in ranked order (everything after the winner).
    pub pruned: Vec<String>,
}

/// Rank candidates best-first and name the winner and the pruned set.
///
/// Ordering is by score descending; ties break on id ascending so a given field always ranks the
/// same way, which keeps the merge decision reproducible. `NaN` scores sort last rather than
/// poisoning the comparison.
pub fn rank_candidates(mut candidates: Vec<CandidateScore>) -> TournamentRanking {
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    let winner = candidates.first().map(|c| c.id.clone());
    let pruned = candidates.iter().skip(1).map(|c| c.id.clone()).collect();
    TournamentRanking {
        ranked: candidates.into_iter().map(|c| c.id).collect(),
        winner,
        pruned,
    }
}

/// The synthetic resolution line injected into the parent history after a tournament.
/// Losing candidates are discarded; this two-line summary is all the parent sees.
pub fn tournament_resolution_line(
    ranking: &TournamentRanking,
    scores: &[CandidateScore],
) -> String {
    let Some(winner) = ranking.winner.as_deref() else {
        return "[Tournament Manager: no candidates executed.]".to_owned();
    };
    let winner_score = scores
        .iter()
        .find(|c| c.id == winner)
        .map(|c| c.score)
        .unwrap_or(0.0);
    let total = ranking.ranked.len();
    if ranking.pruned.is_empty() {
        return format!(
            "[Tournament Manager: {total} candidate executed. Candidate {winner} selected \
             (Score: {winner_score:.2}).]"
        );
    }
    format!(
        "[Tournament Manager: {total} candidates executed. Candidate {winner} selected \
         (Score: {winner_score:.2}). Candidates {} pruned.]",
        ranking.pruned.join(" and ")
    )
}
