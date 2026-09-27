//! Output caps and fail-open sentences for codebase-memory graph results.

use std::time::Duration;

pub(crate) const NOT_CONNECTED: &str =
    "codebase-memory is not connected. Use grep for text search.";
pub(crate) const NO_INDEX: &str = "No codebase-memory index covers this workspace. Use grep.";
pub(crate) const QUERY_FAILED: &str = "codebase-memory query failed. Use grep.";
pub(crate) const QUERY_TIMEOUT: &str = "codebase-memory query timed out. Use grep.";

/// Symbol search row cap (`search_graph` `limit`).
pub(crate) const SEARCH_LIMIT: u32 = 15;
pub(crate) const SEARCH_MAX_BYTES: usize = 2_000;
/// Call-trace row cap (`trace_path` `limit`).
pub(crate) const TRACE_LIMIT: u32 = 30;
pub(crate) const TRACE_MAX_BYTES: usize = 3_000;
/// Model-facing default depth for call traces and blast radius. Hard max is `DEPTH_MAX`.
pub(crate) const TRACE_DEPTH_DEFAULT: u32 = 2;
pub(crate) const DEPTH_MAX: u32 = 3;
/// Blast-radius row cap (`detect_changes` `limit`).
pub(crate) const BLAST_LIMIT: u32 = 40;
pub(crate) const BLAST_MAX_BYTES: usize = 4_000;

pub(crate) const GRAPH_QUERY_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const PROJECT_CACHE_TTL: Duration = Duration::from_secs(60);

pub(crate) fn cap_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    // `get` instead of slicing: this crate denies `clippy::indexing_slicing`.
    // `end` is a char boundary, so the `None` arm does not run.
    let mut out = text.get(..end).unwrap_or("").to_string();
    out.push_str("\n… truncated. Narrow the query.");
    out
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        BLAST_LIMIT, BLAST_MAX_BYTES, DEPTH_MAX, GRAPH_QUERY_TIMEOUT, NO_INDEX, NOT_CONNECTED,
        PROJECT_CACHE_TTL, QUERY_FAILED, QUERY_TIMEOUT, SEARCH_LIMIT, SEARCH_MAX_BYTES,
        TRACE_DEPTH_DEFAULT, TRACE_LIMIT, TRACE_MAX_BYTES,
    };
    use crate::implementations::grok_build::code_graph::cap_text;

    #[test]
    fn cap_text_appends_the_narrow_hint_on_a_char_boundary() {
        let text = "ééééé"; // 5 chars, 10 bytes
        let capped = cap_text(text, 4);
        assert!(capped.starts_with("éé"));
        assert!(capped.ends_with("\n… truncated. Narrow the query."));
        assert!(!capped.contains('\u{fffd}'));
    }

    #[test]
    fn cap_text_returns_text_that_already_fits() {
        assert_eq!(cap_text("ok", 2), "ok");
        assert_eq!(cap_text("ok", 8), "ok");
    }

    #[test]
    fn caps_and_fail_open_sentences_match_the_plan() {
        assert_eq!(
            NOT_CONNECTED,
            "codebase-memory is not connected. Use grep for text search."
        );
        assert_eq!(
            NO_INDEX,
            "No codebase-memory index covers this workspace. Use grep."
        );
        assert_eq!(QUERY_FAILED, "codebase-memory query failed. Use grep.");
        assert_eq!(QUERY_TIMEOUT, "codebase-memory query timed out. Use grep.");
        assert_eq!(SEARCH_LIMIT, 15);
        assert_eq!(SEARCH_MAX_BYTES, 2_000);
        assert_eq!(TRACE_LIMIT, 30);
        assert_eq!(TRACE_MAX_BYTES, 3_000);
        assert_eq!(TRACE_DEPTH_DEFAULT, 2);
        assert_eq!(DEPTH_MAX, 3);
        assert_eq!(BLAST_LIMIT, 40);
        assert_eq!(BLAST_MAX_BYTES, 4_000);
        assert_eq!(GRAPH_QUERY_TIMEOUT, Duration::from_secs(15));
        assert_eq!(PROJECT_CACHE_TTL, Duration::from_secs(60));
    }
}
