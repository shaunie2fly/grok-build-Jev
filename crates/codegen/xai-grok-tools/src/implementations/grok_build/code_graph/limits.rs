//! Output caps and fail-open sentences for codebase-memory graph results.

use std::time::Duration;

pub(crate) const NOT_CONNECTED: &str = "codebase-memory-mcp is not connected — is it registered under that name? Use grep for text search.";
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

/// Appended to any graph body that exceeded its byte cap. Names the levers that actually
/// control size — a bare "narrow the query" left the model guessing which argument to change.
pub(crate) const TRUNCATION_HINT: &str =
    "\n… truncated. Narrow the query: fewer results, lower depth, or a narrower path filter.";

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
    out.push_str(TRUNCATION_HINT);
    out
}

#[cfg(test)]
mod tests {
    use super::cap_text;

    #[test]
    fn cap_text_appends_the_narrow_hint_on_a_char_boundary() {
        let text = "ééééé"; // 5 chars, 10 bytes
        let capped = cap_text(text, 4);
        assert!(capped.starts_with("éé"));
        assert!(capped.contains("truncated"));
        assert!(capped.contains("Narrow the query"));
        assert!(!capped.contains('\u{fffd}'));
    }

    #[test]
    fn cap_text_returns_text_that_already_fits() {
        assert_eq!(cap_text("ok", 2), "ok");
        assert_eq!(cap_text("ok", 8), "ok");
    }
}
