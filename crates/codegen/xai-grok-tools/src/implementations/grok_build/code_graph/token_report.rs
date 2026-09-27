//! Before/after token cost of grep, raw MCP discovery, and the three tools.
//!
//! Not on the tool-call path. `cargo check` does not run the test, so the
//! measurement functions would otherwise be dead.

#![allow(dead_code)]

use xai_token_estimation::estimate_tokens;

use super::cap_text;
use super::limits::{BLAST_MAX_BYTES, SEARCH_MAX_BYTES, TRACE_MAX_BYTES};
use super::tools::{
    BlastRadiusArgs, BlastRadiusTool, SearchSymbolsArgs, SearchSymbolsTool, TraceCallsArgs,
    TraceCallsTool,
};
use crate::types::tool_metadata::ToolMetadata;
use crate::util::mcp_truncate::MCP_MAX_OUTPUT_BYTES;
use crate::DEFAULT_TOOL_OUTPUT_BYTES;

const GREP_LINE: &str = "src/lib.rs:10:fn select_project() {\n";

/// `search_tool` hit descriptions. Property descriptions inside the schemas
/// are the codebase-memory-mcp 0.10.2 `--help` strings from the task brief.
const SEARCH_GRAPH_DESCRIPTION: &str = "Structured search by label, name pattern, file pattern, degree filters. Pagination via limit/offset.";
const TRACE_PATH_DESCRIPTION: &str =
    "BFS traversal — who calls a function and what it calls (alias: `trace_call_path`). Depth 1-5.";
const DETECT_CHANGES_DESCRIPTION: &str =
    "Map git diff to affected symbols + blast radius with risk classification.";

const SEARCH_GRAPH_SCHEMA: &str = r#"{
  "type": "object",
  "required": ["project"],
  "properties": {
    "project": {"type": "string"},
    "query": {"type": "string", "description": "Natural-language or keyword full-text search using BM25 ranking. When provided, name_pattern is ignored."},
    "label": {"type": "string"},
    "name_pattern": {"type": "string"},
    "file_pattern": {"type": "string"},
    "limit": {"type": "integer", "description": "Max results per call. Default 50."},
    "offset": {"type": "integer"},
    "format": {"type": "string", "description": "tree (default) or json."},
    "detail": {"type": "string", "description": "ids or default."},
    "semantic_query": {"type": "array", "description": "ARRAY of keyword strings. Requires moderate/full index mode."}
  }
}"#;

const TRACE_PATH_SCHEMA: &str = r#"{
  "type": "object",
  "required": ["project", "function_name"],
  "properties": {
    "function_name": {"type": "string"},
    "project": {"type": "string"},
    "direction": {"type": "string"},
    "depth": {"type": "integer"},
    "limit": {"type": "integer", "description": "Rows per page."},
    "mode": {"type": "string", "description": "calls, data_flow, or cross_service."},
    "include_tests": {"type": "boolean"},
    "format": {"type": "string"},
    "include_evidence": {"type": "boolean", "description": "Adds strategy and confidence columns."},
    "risk_labels": {"type": "boolean"}
  }
}"#;

const DETECT_CHANGES_SCHEMA: &str = r#"{
  "type": "object",
  "required": ["project"],
  "properties": {
    "project": {"type": "string"},
    "scope": {"type": "string", "description": "files, or impact (default)."},
    "direction": {"type": "string", "description": "inbound (default) is the blast radius."},
    "depth": {"type": "integer"},
    "limit": {"type": "integer", "description": "Per-symbol impacted rows. Default 200."},
    "base_branch": {"type": "string"},
    "since": {"type": "string"},
    "format": {"type": "string"}
  }
}"#;

pub fn report() -> String {
    let grep_tokens = estimate_tokens(&grep_body());
    let search_tokens = estimate_tokens(&search_after());
    let trace_tokens = estimate_tokens(&trace_after());
    let blast_tokens = estimate_tokens(&blast_after());
    let standing = standing_tokens();
    let mcp_before = estimate_tokens(&discovery_body()) + estimate_tokens(&mcp_body());
    let idle_after = 8 * standing;

    [
        "estimator: xai_token_estimation::estimate_tokens (bytes/4)".to_string(),
        "session: 8 turns; descriptions are resent every turn; lookups are 0, 1, or 3".to_string(),
        String::new(),
        format!(
            "{:<26}   {:>6}   {:>5}   {:>5}",
            "row", "before", "after", "delta"
        ),
        numeric_row("grep content cap", grep_tokens, search_tokens),
        numeric_row("trace cap", grep_tokens, trace_tokens),
        numeric_row("blast cap", grep_tokens, blast_tokens),
        numeric_row("mcp discovery plus body", mcp_before, search_tokens),
        row("session 8 turns, 0 lookups", 0, idle_after, "overhead"),
        numeric_row(
            "session 8 turns, 1 lookup",
            grep_tokens,
            search_tokens + idle_after,
        ),
        numeric_row(
            "session 8 turns, 3 lookups",
            grep_tokens * 3,
            search_tokens * 3 + idle_after,
        ),
    ]
    .join("\n")
}

fn grep_body() -> String {
    let mut body = String::new();
    while body.len() < DEFAULT_TOOL_OUTPUT_BYTES {
        body.push_str(GREP_LINE);
    }
    truncate_on_char_boundary(&body, DEFAULT_TOOL_OUTPUT_BYTES)
}

fn search_after() -> String {
    cap_text(&grep_body(), SEARCH_MAX_BYTES)
}

fn trace_after() -> String {
    cap_text(&grep_body(), TRACE_MAX_BYTES)
}

fn blast_after() -> String {
    cap_text(&grep_body(), BLAST_MAX_BYTES)
}

fn mcp_body() -> String {
    truncate_on_char_boundary(&grep_body(), MCP_MAX_OUTPUT_BYTES)
}

fn discovery_body() -> String {
    let hits = [
        (
            "codebase-memory-mcp__search_graph",
            SEARCH_GRAPH_DESCRIPTION,
            SEARCH_GRAPH_SCHEMA,
        ),
        (
            "codebase-memory-mcp__trace_path",
            TRACE_PATH_DESCRIPTION,
            TRACE_PATH_SCHEMA,
        ),
        (
            "codebase-memory-mcp__detect_changes",
            DETECT_CHANGES_DESCRIPTION,
            DETECT_CHANGES_SCHEMA,
        ),
    ];
    let mut out = String::from("{\n  \"results\": [\n");
    for (index, (name, description, schema)) in hits.iter().enumerate() {
        if index > 0 {
            out.push_str(",\n");
        }
        out.push_str("    {\n      \"tool_name\": ");
        out.push_str(&json_string(name));
        out.push_str(",\n      \"description\": ");
        out.push_str(&json_string(description));
        out.push_str(",\n      \"input_schema\": ");
        out.push_str(schema);
        out.push_str("\n    }");
    }
    out.push_str("\n  ]\n}");
    out
}

fn standing_tokens() -> u64 {
    let search = definition_tokens(
        "search_symbols",
        SearchSymbolsTool.description_template(),
        serde_json::to_value(schemars::schema_for!(SearchSymbolsArgs)).expect("search schema"),
    );
    let trace = definition_tokens(
        "trace_calls",
        TraceCallsTool.description_template(),
        serde_json::to_value(schemars::schema_for!(TraceCallsArgs)).expect("trace schema"),
    );
    let blast = definition_tokens(
        "blast_radius",
        BlastRadiusTool.description_template(),
        serde_json::to_value(schemars::schema_for!(BlastRadiusArgs)).expect("blast schema"),
    );
    search + trace + blast
}

fn definition_tokens(name: &str, description: &str, parameters: serde_json::Value) -> u64 {
    let definition = serde_json::json!({
        "name": name,
        "description": description,
        "parameters": parameters
    });
    estimate_tokens(&definition.to_string())
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("json string")
}

fn truncate_on_char_boundary(text: &str, max_bytes: usize) -> String {
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or("").to_string()
}

fn row(label: &str, before: u64, after: u64, delta: &str) -> String {
    format!("{label:<26}   {before:>6}   {after:>5}   {delta:>5}")
}

fn numeric_row(label: &str, before: u64, after: u64) -> String {
    let delta = if after > before {
        format!("-{}", after - before)
    } else {
        (before - after).to_string()
    };
    row(label, before, after, &delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_report_shows_before_and_after() {
        let report = report();
        println!("{report}");
        assert!(report.contains("estimator: xai_token_estimation::estimate_tokens (bytes/4)"));
        assert!(report.contains("grep content cap"));
        assert!(report.contains("mcp discovery plus body"));
        assert!(report.contains("session 8 turns, 0 lookups"));
        assert!(report.contains("session 8 turns, 1 lookup"));
        assert!(report.contains("session 8 turns, 3 lookups"));
        assert!(report.contains("overhead"));

        let grep_tokens = estimate_tokens(&grep_body());
        let search_tokens = estimate_tokens(&search_after());
        assert!(
            grep_tokens >= 8_000 && grep_tokens - search_tokens >= 8_000,
            "one saturated grep must save at least 8000 tokens versus search_symbols\n{report}"
        );

        let standing = standing_tokens();
        assert!(
            standing * 8 < grep_tokens - search_tokens,
            "eight turns of the three definitions must cost less than one avoided grep\n{report}"
        );

        let mcp_before = estimate_tokens(&discovery_body()) + estimate_tokens(&mcp_body());
        assert!(
            mcp_before > search_tokens + standing,
            "one raw MCP lookup must cost more than one native lookup plus the definitions\n{report}"
        );

        let idle_after = 8 * standing;
        let one_before = grep_tokens;
        let one_after = search_tokens + idle_after;
        assert!(
            one_after < one_before,
            "one lookup in eight turns must be cheaper\n{report}"
        );
        let three_before = grep_tokens * 3;
        let three_after = search_tokens * 3 + idle_after;
        assert!(
            three_after < three_before,
            "three lookups in eight turns must be cheaper\n{report}"
        );
    }
}
