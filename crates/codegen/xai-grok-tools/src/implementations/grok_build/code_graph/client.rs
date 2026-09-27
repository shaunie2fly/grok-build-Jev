//! Dispatch capped codebase-memory queries through the session's MCP tools.
//!
//! One project name is cached per session cwd. A stale name is dropped and the
//! call is tried once more. Failures come back as sentences, not `ToolError`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Instant;

use serde::Serialize;
use serde_json::Value;

use super::limits::{
    BLAST_LIMIT, BLAST_MAX_BYTES, DEPTH_MAX, GRAPH_QUERY_TIMEOUT, NO_INDEX, NOT_CONNECTED,
    PROJECT_CACHE_TTL, QUERY_FAILED, QUERY_TIMEOUT, SEARCH_LIMIT, SEARCH_MAX_BYTES, TRACE_LIMIT,
    TRACE_MAX_BYTES,
};
use super::{cap_text, parse_projects, select_project};
use crate::implementations::use_tool::dispatch_mcp_tool;
use crate::types::output::{MCPOutputDetails, ToolOutput};
use crate::types::tool_metadata::{resolve_cwd, shared_resources};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchRequest {
    pub(crate) name_pattern: Option<String>,
    pub(crate) query: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) file_pattern: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TraceRequest {
    pub(crate) function_name: String,
    pub(crate) direction: TraceDirection,
    pub(crate) depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlastRequest {
    pub(crate) since: Option<String>,
    pub(crate) scope: BlastScope,
    pub(crate) direction: TraceDirection,
    pub(crate) depth: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TraceDirection {
    Inbound,
    Outbound,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BlastScope {
    Impact,
    Files,
}

enum QueryOutcome {
    Text(String),
    Sentence(&'static str),
    UnknownProject,
}

fn project_cache() -> &'static tokio::sync::Mutex<HashMap<PathBuf, (String, Instant)>> {
    static CACHE: LazyLock<tokio::sync::Mutex<HashMap<PathBuf, (String, Instant)>>> =
        LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));
    &CACHE
}

pub(crate) async fn search_symbols(
    ctx: &xai_tool_runtime::ToolCallContext,
    request: SearchRequest,
) -> String {
    call_with_project(
        ctx,
        "search_graph",
        search_args(&request),
        SEARCH_MAX_BYTES,
        "search_symbols",
    )
    .await
}

pub(crate) async fn trace_calls(
    ctx: &xai_tool_runtime::ToolCallContext,
    request: TraceRequest,
) -> String {
    call_with_project(
        ctx,
        "trace_path",
        trace_args(&request),
        TRACE_MAX_BYTES,
        "trace_calls",
    )
    .await
}

pub(crate) async fn blast_radius(
    ctx: &xai_tool_runtime::ToolCallContext,
    request: BlastRequest,
) -> String {
    call_with_project(
        ctx,
        "detect_changes",
        blast_args(&request),
        BLAST_MAX_BYTES,
        "blast_radius",
    )
    .await
}

async fn call_with_project(
    ctx: &xai_tool_runtime::ToolCallContext,
    tool: &str,
    args: Value,
    max_bytes: usize,
    caller: &str,
) -> String {
    let cwd = match load_cwd(ctx).await {
        Ok(cwd) => cwd,
        Err(sentence) => return sentence.to_string(),
    };
    match run_query(ctx, &cwd, tool, &args, caller).await {
        QueryOutcome::Text(text) => cap_text(&text, max_bytes),
        QueryOutcome::Sentence(sentence) => sentence.to_string(),
        QueryOutcome::UnknownProject => {
            forget_project(&cwd).await;
            match run_query(ctx, &cwd, tool, &args, caller).await {
                QueryOutcome::Text(text) => cap_text(&text, max_bytes),
                // Keep the specific sentence. Collapsing these into `QUERY_FAILED` told the
                // model "query failed" when the truth was "the server is gone" or "nothing
                // indexes this workspace" — two answers that need different follow-up.
                QueryOutcome::Sentence(sentence) => sentence.to_string(),
                QueryOutcome::UnknownProject => QUERY_FAILED.to_string(),
            }
        }
    }
}

async fn run_query(
    ctx: &xai_tool_runtime::ToolCallContext,
    cwd: &std::path::Path,
    tool: &str,
    args: &Value,
    caller: &str,
) -> QueryOutcome {
    let project = match resolve_project(ctx, cwd, caller).await {
        Ok(project) => project,
        Err(sentence) => return QueryOutcome::Sentence(sentence),
    };
    let mut args = args.clone();
    insert_project(&mut args, &project);
    dispatch_tool(ctx, tool, args, caller).await
}

async fn load_cwd(ctx: &xai_tool_runtime::ToolCallContext) -> Result<PathBuf, &'static str> {
    let resources = shared_resources(ctx).map_err(|_| NO_INDEX)?;
    resolve_cwd(ctx, &resources).await.map_err(|_| NO_INDEX)
}

async fn resolve_project(
    ctx: &xai_tool_runtime::ToolCallContext,
    cwd: &std::path::Path,
    caller: &str,
) -> Result<String, &'static str> {
    if let Some(name) = cached_project(cwd).await {
        return Ok(name);
    }
    let listed = match dispatch_tool(ctx, "list_projects", serde_json::json!({}), caller).await {
        QueryOutcome::Text(text) => text,
        QueryOutcome::Sentence(sentence) => return Err(sentence),
        QueryOutcome::UnknownProject => return Err(QUERY_FAILED),
    };
    let projects = parse_projects(&listed).map_err(|_| NO_INDEX)?;
    let name = select_project(cwd, &projects).ok_or(NO_INDEX)?;
    remember_project(cwd, &name).await;
    Ok(name)
}

async fn dispatch_tool(
    ctx: &xai_tool_runtime::ToolCallContext,
    tool: &str,
    args: Value,
    caller: &str,
) -> QueryOutcome {
    let name = qualified(tool);
    match tokio::time::timeout(
        GRAPH_QUERY_TIMEOUT,
        dispatch_mcp_tool(ctx, &name, args, caller),
    )
    .await
    {
        Ok(Ok(output)) => match graph_body(&output) {
            Some(GraphBody::McpError(text)) if is_unknown_project(&text) => {
                QueryOutcome::UnknownProject
            }
            Some(GraphBody::Text(text) | GraphBody::McpError(text)) => QueryOutcome::Text(text),
            None => QueryOutcome::Sentence(QUERY_FAILED),
        },
        Ok(Err(err)) if is_not_connected(&err) => QueryOutcome::Sentence(NOT_CONNECTED),
        Ok(Err(err)) if is_unknown_project(&err.detail) => QueryOutcome::UnknownProject,
        Ok(Err(_)) => QueryOutcome::Sentence(QUERY_FAILED),
        Err(_) => QueryOutcome::Sentence(QUERY_TIMEOUT),
    }
}

fn qualified(tool: &str) -> String {
    format!("codebase-memory-mcp__{tool}")
}

enum GraphBody {
    /// `ToolOutput::Text` or an MCP okay body. Never an unknown-project signal.
    Text(String),
    /// MCP `isError` body. May be a stale project name.
    McpError(String),
}

/// Success text is returned as-is. Only an MCP error body can be a stale project.
fn graph_body(output: &ToolOutput) -> Option<GraphBody> {
    match output {
        ToolOutput::Text(text) => Some(GraphBody::Text(text.text.clone())),
        ToolOutput::MCP(mcp) => match mcp.output() {
            MCPOutputDetails::OkayOutput(text) => Some(GraphBody::Text(text.clone())),
            MCPOutputDetails::Error(text) => Some(GraphBody::McpError(text.clone())),
        },
        _ => None,
    }
}

fn is_not_connected(err: &xai_tool_runtime::ToolError) -> bool {
    err.kind == xai_tool_runtime::ToolErrorKind::NotFound
        || err.detail.contains("inner_dispatch not set")
        || err.detail.contains("not a valid MCP tool name")
}

/// `codebase-memory-mcp` 0.10.2 reports a stale or rejected name in these two forms. Checked only
/// on `ToolError` detail and MCP error bodies, never on a successful graph result — a graph row
/// may legitimately contain either phrase.
const UNKNOWN_PROJECT_MARKERS: [&str; 2] = ["project not found", "invalid project name"];

/// The exact 0.10.2 body, captured from a live call with a bogus project. If a server upgrade
/// rephrases the error, `the_live_unknown_project_body_is_still_recognised` fails loudly instead
/// of the marker silently falling through to `QUERY_FAILED` and losing the one automatic retry.
#[cfg(test)]
const OBSERVED_UNKNOWN_PROJECT_BODY: &str = "{\"error\":\"project not found or not indexed\",\"hint\":\"Use list_projects to see all indexed projects, then pass it as the \\\"project\\\" argument.\",\"available_projects\":[\"mnt-data-repos-grok-build-Jev\"],\"count\":1}";

fn is_unknown_project(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    UNKNOWN_PROJECT_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

fn clamp_depth(depth: u32) -> u32 {
    depth.clamp(1, DEPTH_MAX)
}

fn search_args(request: &SearchRequest) -> Value {
    let mut args = serde_json::Map::new();
    insert_some(&mut args, "name_pattern", &request.name_pattern);
    insert_some(&mut args, "query", &request.query);
    insert_some(&mut args, "label", &request.label);
    insert_some(&mut args, "file_pattern", &request.file_pattern);
    args.insert("limit".to_string(), serde_json::json!(SEARCH_LIMIT));
    args.insert("offset".to_string(), serde_json::json!(0));
    args.insert("format".to_string(), Value::String("tree".to_string()));
    args.insert("detail".to_string(), Value::String("default".to_string()));
    Value::Object(args)
}

fn trace_args(request: &TraceRequest) -> Value {
    let mut args = serde_json::Map::new();
    args.insert(
        "function_name".to_string(),
        Value::String(request.function_name.clone()),
    );
    args.insert("direction".to_string(), json_value(request.direction));
    args.insert(
        "depth".to_string(),
        serde_json::json!(clamp_depth(request.depth)),
    );
    args.insert("limit".to_string(), serde_json::json!(TRACE_LIMIT));
    args.insert("format".to_string(), Value::String("tree".to_string()));
    args.insert("mode".to_string(), Value::String("calls".to_string()));
    args.insert("include_tests".to_string(), Value::Bool(false));
    Value::Object(args)
}

fn blast_args(request: &BlastRequest) -> Value {
    let mut args = serde_json::Map::new();
    insert_some(&mut args, "since", &request.since);
    args.insert("scope".to_string(), json_value(request.scope));
    args.insert("direction".to_string(), json_value(request.direction));
    args.insert(
        "depth".to_string(),
        serde_json::json!(clamp_depth(request.depth)),
    );
    args.insert("limit".to_string(), serde_json::json!(BLAST_LIMIT));
    args.insert("format".to_string(), Value::String("tree".to_string()));
    Value::Object(args)
}

fn insert_some(args: &mut serde_json::Map<String, Value>, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        args.insert(key.to_string(), Value::String(value.clone()));
    }
}

fn insert_project(args: &mut Value, project: &str) {
    if let Some(map) = args.as_object_mut() {
        map.insert("project".to_string(), Value::String(project.to_string()));
    }
}

fn json_value(value: impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

async fn cached_project(cwd: &std::path::Path) -> Option<String> {
    let cache = project_cache().lock().await;
    let (name, stored_at) = cache.get(cwd)?;
    (stored_at.elapsed() < PROJECT_CACHE_TTL).then(|| name.clone())
}

async fn remember_project(cwd: &std::path::Path, name: &str) {
    project_cache()
        .lock()
        .await
        .insert(cwd.to_path_buf(), (name.to_string(), Instant::now()));
}

async fn forget_project(cwd: &std::path::Path) {
    project_cache().lock().await.remove(cwd);
}

#[cfg(test)]
pub(crate) async fn clear_project_cache_for_test() {
    project_cache().lock().await.clear();
}

#[cfg(test)]
pub(crate) async fn remember_expired_project_for_test(cwd: &std::path::Path, name: &str) {
    let stored_at = Instant::now()
        .checked_sub(PROJECT_CACHE_TTL + std::time::Duration::from_secs(1))
        .expect("monotonic clock is older than the project cache ttl");
    project_cache()
        .lock()
        .await
        .insert(cwd.to_path_buf(), (name.to_string(), stored_at));
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, LazyLock, Mutex};

    use serde_json::Value;

    use super::clear_project_cache_for_test;
    use super::{
        BlastRequest, BlastScope, SearchRequest, TraceDirection, TraceRequest, blast_radius,
        search_symbols, trace_calls,
    };
    use crate::implementations::grok_build::code_graph::limits::{
        NO_INDEX, NOT_CONNECTED, QUERY_FAILED, TRUNCATION_HINT,
    };
    use crate::types::output::{MCPOutput, SearchToolOutput, ToolOutput};
    use crate::types::resources::{Cwd, InnerDispatch, Resources};
    use crate::types::tool_metadata::test_ctx;

    static TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));

    async fn begin() -> tokio::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().await;
        clear_project_cache_for_test().await;
        guard
    }

    #[derive(Clone, Debug)]
    struct RecordedCall {
        name: String,
        args: Value,
    }

    #[derive(Clone)]
    struct FakeDispatch {
        calls: Arc<Mutex<Vec<RecordedCall>>>,
        responses: Arc<Mutex<VecDeque<Result<ToolOutput, xai_tool_runtime::ToolError>>>>,
    }

    impl FakeDispatch {
        fn with_responses(responses: Vec<ToolOutput>) -> Self {
            Self::scripted(responses.into_iter().map(Ok).collect())
        }

        fn scripted(responses: Vec<Result<ToolOutput, xai_tool_runtime::ToolError>>) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            }
        }

        fn call(&self, index: usize) -> RecordedCall {
            self.calls
                .lock()
                .expect("calls")
                .get(index)
                .unwrap_or_else(|| panic!("no recorded call at {index}"))
                .clone()
        }

        fn len(&self) -> usize {
            self.calls.lock().expect("calls").len()
        }
    }

    #[async_trait::async_trait]
    impl xai_tool_runtime::ToolDispatch for FakeDispatch {
        async fn call(
            &self,
            tool_id: xai_tool_protocol::ToolId,
            args: Value,
            _ctx: xai_tool_runtime::ToolCallContext,
        ) -> xai_tool_runtime::ToolStream<xai_tool_runtime::TypedToolOutput> {
            self.calls.lock().expect("calls").push(RecordedCall {
                name: tool_id.as_str().to_string(),
                args,
            });
            let response = self
                .responses
                .lock()
                .expect("responses")
                .pop_front()
                .expect("scripted response");
            let terminal = match response {
                Ok(output) => {
                    let value = serde_json::to_value(&output).expect("output json");
                    Ok(xai_tool_runtime::TypedToolOutput::from_value(
                        tool_id, value,
                    ))
                }
                Err(err) => Err(err),
            };
            xai_tool_runtime::terminal_only(terminal)
        }
    }

    fn text_output(text: &str) -> ToolOutput {
        ToolOutput::Text(text.into())
    }

    fn mcp_error(text: &str) -> ToolOutput {
        ToolOutput::MCP(MCPOutput::errored(
            "codebase-memory-mcp__search_graph".into(),
            "codebase-memory-mcp".into(),
            text.into(),
        ))
    }

    fn list_projects_json(name: &str, root: &str, nodes: u64) -> ToolOutput {
        let body = serde_json::json!({
            "projects": [{
                "name": name,
                "root_path": root,
                "nodes": nodes,
                "size_bytes": 0
            }]
        });
        text_output(&body.to_string())
    }

    fn empty_resources(cwd: &Path) -> crate::types::resources::SharedResources {
        let mut resources = Resources::new();
        resources.insert(Cwd(cwd.to_path_buf()));
        resources.into_shared()
    }

    fn ctx_with_cwd_and_dispatch(
        cwd: &Path,
        fake: FakeDispatch,
    ) -> xai_tool_runtime::ToolCallContext {
        let mut ctx = test_ctx(empty_resources(cwd));
        ctx.extensions.insert(InnerDispatch(Arc::new(fake)));
        ctx
    }

    fn json_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
        args.get(key).and_then(Value::as_str)
    }

    fn json_u64(args: &Value, key: &str) -> Option<u64> {
        args.get(key).and_then(Value::as_u64)
    }

    fn symbol_request() -> SearchRequest {
        SearchRequest {
            name_pattern: Some("grok".into()),
            query: None,
            label: None,
            file_pattern: None,
        }
    }

    #[tokio::test]
    async fn search_symbols_sends_a_capped_tree_query_for_the_covering_project() {
        let _guard = begin().await;
        let cwd = PathBuf::from("/repo");
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 10),
            text_output("fn grok\nsrc/lib.rs:1"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(&cwd, fake.clone());
        let out = search_symbols(
            &ctx,
            SearchRequest {
                name_pattern: Some("grok".into()),
                query: None,
                label: Some("Function".into()),
                file_pattern: None,
            },
        )
        .await;
        assert_eq!(out, "fn grok\nsrc/lib.rs:1");
        let call = fake.call(1);
        assert_eq!(call.name, "codebase-memory-mcp__search_graph");
        assert_eq!(json_str(&call.args, "project"), Some("grok"));
        assert_eq!(json_str(&call.args, "name_pattern"), Some("grok"));
        assert_eq!(json_str(&call.args, "label"), Some("Function"));
        assert_eq!(json_u64(&call.args, "limit"), Some(15));
        assert_eq!(json_str(&call.args, "format"), Some("tree"));
        assert!(call.args.get("semantic_query").is_none());
        assert_eq!(json_u64(&call.args, "offset"), Some(0));
        assert_eq!(json_str(&call.args, "detail"), Some("default"));
        assert!(call.args.get("query").is_none());
        assert!(call.args.get("file_pattern").is_none());
        for key in [
            "fields",
            "qn_pattern",
            "relationship",
            "cursor",
            "edge_types",
            "risk_labels",
            "include_evidence",
            "base_branch",
        ] {
            assert!(call.args.get(key).is_none(), "{key}");
        }
    }

    #[tokio::test]
    async fn missing_server_returns_the_not_connected_sentence() {
        let _guard = begin().await;
        let ctx = test_ctx(empty_resources(Path::new("/repo")));
        let out = trace_calls(
            &ctx,
            TraceRequest {
                function_name: "select_project".into(),
                direction: TraceDirection::Inbound,
                depth: 9,
            },
        )
        .await;
        assert_eq!(out, NOT_CONNECTED);
    }

    #[tokio::test]
    async fn trace_depth_is_clamped_and_mode_is_calls() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 10),
            text_output("caller"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let _ = trace_calls(
            &ctx,
            TraceRequest {
                function_name: "select_project".into(),
                direction: TraceDirection::Both,
                depth: 9,
            },
        )
        .await;
        let call = fake.call(1);
        let args = &call.args;
        assert_eq!(json_u64(args, "depth"), Some(3));
        assert_eq!(json_str(args, "direction"), Some("both"));
        assert_eq!(json_str(args, "mode"), Some("calls"));
        assert_eq!(
            args.get("include_tests").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(json_u64(args, "limit"), Some(30));
        assert_eq!(json_str(args, "format"), Some("tree"));
    }

    #[tokio::test]
    async fn two_indexes_of_one_root_use_the_larger_node_count() {
        let _guard = begin().await;
        let body = r#"{"projects":[
        {"name":"mnt-data-repos-grok-build-Jev","root_path":"/repo","nodes":133436,"size_bytes":695140352},
        {"name":"grok-build-Jev","root_path":"/repo","nodes":133395,"size_bytes":595066880}
    ]}"#;
        let fake = FakeDispatch::with_responses(vec![text_output(body), text_output("ok")]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let _ = blast_radius(
            &ctx,
            BlastRequest {
                since: Some("HEAD~1".into()),
                scope: BlastScope::Impact,
                direction: TraceDirection::Inbound,
                depth: 2,
            },
        )
        .await;
        let call = fake.call(1);
        let args = &call.args;
        assert_eq!(
            json_str(args, "project"),
            Some("mnt-data-repos-grok-build-Jev")
        );
        assert_eq!(json_str(args, "since"), Some("HEAD~1"));
        assert_eq!(json_str(args, "scope"), Some("impact"));
        assert_eq!(json_u64(args, "limit"), Some(40));
        assert!(args.get("base_branch").is_none());
        assert_eq!(json_str(args, "format"), Some("tree"));
        assert_eq!(json_str(args, "direction"), Some("inbound"));
        assert_eq!(json_u64(args, "depth"), Some(2));
    }

    #[tokio::test]
    async fn trace_depth_below_one_clamps_to_one() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 1),
            text_output("callee"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let _ = trace_calls(
            &ctx,
            TraceRequest {
                function_name: "select_project".into(),
                direction: TraceDirection::Outbound,
                depth: 0,
            },
        )
        .await;
        let args = &fake.call(1).args;
        assert_eq!(json_u64(args, "depth"), Some(1));
        assert_eq!(json_str(args, "direction"), Some("outbound"));
    }

    #[tokio::test]
    async fn omitted_since_is_not_sent() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 1),
            text_output("ok"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let _ = blast_radius(
            &ctx,
            BlastRequest {
                since: None,
                scope: BlastScope::Files,
                direction: TraceDirection::Outbound,
                depth: 1,
            },
        )
        .await;
        let args = &fake.call(1).args;
        assert!(args.get("since").is_none());
        assert_eq!(json_str(args, "scope"), Some("files"));
        assert_eq!(json_str(args, "direction"), Some("outbound"));
        assert!(args.get("base_branch").is_none());
    }

    #[tokio::test]
    async fn search_forwards_query_and_file_pattern() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 1),
            text_output("hit"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let out = search_symbols(
            &ctx,
            SearchRequest {
                name_pattern: None,
                query: Some("symbol table".into()),
                label: None,
                file_pattern: Some("*.rs".into()),
            },
        )
        .await;
        assert_eq!(out, "hit");
        let args = &fake.call(1).args;
        assert_eq!(json_str(args, "query"), Some("symbol table"));
        assert_eq!(json_str(args, "file_pattern"), Some("*.rs"));
        assert!(args.get("name_pattern").is_none());
    }

    #[tokio::test]
    async fn no_covering_project_returns_no_index() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![list_projects_json("other", "/somewhere", 1)]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let out = search_symbols(&ctx, symbol_request()).await;
        assert_eq!(out, NO_INDEX);
        assert_eq!(fake.len(), 1);
        assert_eq!(fake.call(0).name, "codebase-memory-mcp__list_projects");
        assert_eq!(fake.call(0).args, serde_json::json!({}));
    }

    #[tokio::test]
    async fn missing_cwd_returns_no_index() {
        let _guard = begin().await;
        let ctx = test_ctx(Resources::new().into_shared());
        let out = search_symbols(&ctx, symbol_request()).await;
        assert_eq!(out, NO_INDEX);
    }

    #[tokio::test]
    async fn not_found_tool_returns_not_connected() {
        let _guard = begin().await;
        let fake = FakeDispatch::scripted(vec![Err(xai_tool_runtime::ToolError::not_found(
            xai_tool_protocol::ToolId::new("codebase-memory-mcp__list_projects").expect("id"),
            "Tool not found",
        ))]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake);
        let out = trace_calls(
            &ctx,
            TraceRequest {
                function_name: "select_project".into(),
                direction: TraceDirection::Inbound,
                depth: 2,
            },
        )
        .await;
        assert_eq!(out, NOT_CONNECTED);
    }

    #[tokio::test]
    async fn invalid_mcp_name_returns_not_connected() {
        let _guard = begin().await;
        let fake =
            FakeDispatch::scripted(vec![Err(xai_tool_runtime::ToolError::invalid_arguments(
                "'trace_calls' is not a valid MCP tool name. Use search_tool.",
            ))]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake);
        let out = trace_calls(
            &ctx,
            TraceRequest {
                function_name: "select_project".into(),
                direction: TraceDirection::Inbound,
                depth: 1,
            },
        )
        .await;
        assert_eq!(out, NOT_CONNECTED);
    }

    #[tokio::test]
    async fn other_dispatch_error_returns_query_failed() {
        let _guard = begin().await;
        let fake = FakeDispatch::scripted(vec![Err(
            xai_tool_runtime::ToolError::invalid_arguments("local validation failed"),
        )]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake);
        let out = search_symbols(&ctx, symbol_request()).await;
        assert_eq!(out, QUERY_FAILED);
    }

    #[tokio::test]
    async fn mcp_text_body_is_capped_and_other_variants_fail_open() {
        let _guard = begin().await;
        let body = "y".repeat(2_050);
        let mcp = ToolOutput::MCP(MCPOutput::okay_output(
            "codebase-memory-mcp__search_graph".into(),
            "codebase-memory-mcp".into(),
            body,
        ));
        let fake = FakeDispatch::with_responses(vec![list_projects_json("grok", "/repo", 1), mcp]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake);
        let out = search_symbols(&ctx, symbol_request()).await;
        assert!(out.ends_with(TRUNCATION_HINT));
        assert!(out.starts_with("yyyy"));
        assert_eq!(out.len(), 2_000 + TRUNCATION_HINT.len());

        clear_project_cache_for_test().await;
        let other = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 1),
            ToolOutput::SearchTool(SearchToolOutput {
                result_count: 1,
                content: "hidden".into(),
            }),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), other);
        let out = search_symbols(
            &ctx,
            SearchRequest {
                name_pattern: None,
                query: Some("grok".into()),
                label: None,
                file_pattern: None,
            },
        )
        .await;
        assert_eq!(out, QUERY_FAILED);
    }

    #[tokio::test]
    async fn mcp_error_text_that_is_not_an_unknown_project_is_returned() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 1),
            ToolOutput::MCP(MCPOutput::errored(
                "codebase-memory-mcp__trace_path".into(),
                "codebase-memory-mcp".into(),
                "database is locked".into(),
            )),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake);
        let out = trace_calls(
            &ctx,
            TraceRequest {
                function_name: "select_project".into(),
                direction: TraceDirection::Inbound,
                depth: 2,
            },
        )
        .await;
        assert_eq!(out, "database is locked");
    }

    #[tokio::test]
    async fn graph_text_that_mentions_an_unknown_project_is_returned() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 1),
            text_output("unknown project in the tree"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let out = search_symbols(&ctx, symbol_request()).await;
        assert_eq!(out, "unknown project in the tree");
        assert_ne!(out, QUERY_FAILED);
        assert_eq!(fake.len(), 2);
    }

    #[tokio::test]
    async fn unknown_project_retries_once_with_a_fresh_list() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("stale", "/repo", 1),
            mcp_error(r#"{"error":"project not found"}"#),
            list_projects_json("grok", "/repo", 10),
            text_output("fn grok"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let out = search_symbols(&ctx, symbol_request()).await;
        assert_eq!(out, "fn grok");
        assert_eq!(fake.len(), 4);
        assert_eq!(json_str(&fake.call(1).args, "project"), Some("stale"));
        assert_eq!(fake.call(2).name, "codebase-memory-mcp__list_projects");
        assert_eq!(json_str(&fake.call(3).args, "project"), Some("grok"));
    }

    /// Regression guard for the retry arm that forwards `Sentence`. Reverting that arm to
    /// `_ => QUERY_FAILED` used to pass the suite: the other three retry tests all end in `Text`
    /// or a second `UnknownProject`, so nothing reached it. The distinguishing case is a retry
    /// that fails for a *different* reason than the stale project it was retrying past.
    #[tokio::test]
    async fn a_retry_that_fails_differently_keeps_its_own_sentence() {
        let _guard = begin().await;
        let tool_id =
            xai_tool_protocol::ToolId::new("codebase-memory-mcp__search_graph").expect("id");
        let fake = FakeDispatch::scripted(vec![
            Ok(list_projects_json("stale", "/repo", 1)),
            Ok(mcp_error(r#"{"error":"project not found"}"#)),
            Ok(list_projects_json("grok", "/repo", 10)),
            Err(xai_tool_runtime::ToolError::not_found(
                tool_id,
                "Tool not found",
            )),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let out = search_symbols(&ctx, symbol_request()).await;
        assert_eq!(out, NOT_CONNECTED);
        assert_ne!(out, QUERY_FAILED);
        assert_eq!(fake.len(), 4);
    }

    #[tokio::test]
    async fn unknown_project_tool_error_retries_once() {
        let _guard = begin().await;
        let tool_id =
            xai_tool_protocol::ToolId::new("codebase-memory-mcp__trace_path").expect("id");
        let fake = FakeDispatch::scripted(vec![
            Ok(list_projects_json("stale", "/repo", 1)),
            Err(xai_tool_runtime::ToolError::execution(
                tool_id,
                r#"{"error":"project not found"}"#,
            )),
            Ok(list_projects_json("grok", "/repo", 8)),
            Ok(text_output("caller")),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let out = trace_calls(
            &ctx,
            TraceRequest {
                function_name: "select_project".into(),
                direction: TraceDirection::Both,
                depth: 2,
            },
        )
        .await;
        assert_eq!(out, "caller");
        assert_eq!(json_str(&fake.call(3).args, "project"), Some("grok"));
    }

    #[tokio::test]
    async fn unknown_project_a_second_time_is_query_failed() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("stale", "/repo", 1),
            mcp_error("project not found or not indexed"),
            list_projects_json("still-stale", "/repo", 2),
            mcp_error("invalid project name"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let out = blast_radius(
            &ctx,
            BlastRequest {
                since: None,
                scope: BlastScope::Impact,
                direction: TraceDirection::Both,
                depth: 2,
            },
        )
        .await;
        assert_eq!(out, QUERY_FAILED);
        assert_eq!(fake.len(), 4);
    }

    #[tokio::test]
    async fn cached_project_skips_list_projects_until_cleared() {
        let _guard = begin().await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 4),
            text_output("one"),
            text_output("two"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let request = SearchRequest {
            name_pattern: Some("a".into()),
            query: None,
            label: None,
            file_pattern: None,
        };
        assert_eq!(search_symbols(&ctx, request.clone()).await, "one");
        assert_eq!(search_symbols(&ctx, request).await, "two");
        assert_eq!(fake.len(), 3);
        assert_eq!(fake.call(0).name, "codebase-memory-mcp__list_projects");
        assert_eq!(fake.call(2).name, "codebase-memory-mcp__search_graph");
        assert_eq!(json_str(&fake.call(2).args, "project"), Some("grok"));
    }

    #[tokio::test]
    async fn expired_project_cache_lists_again() {
        let _guard = begin().await;
        super::remember_expired_project_for_test(Path::new("/repo"), "stale").await;
        let fake = FakeDispatch::with_responses(vec![
            list_projects_json("grok", "/repo", 3),
            text_output("fresh"),
        ]);
        let ctx = ctx_with_cwd_and_dispatch(Path::new("/repo"), fake.clone());
        let out = search_symbols(&ctx, symbol_request()).await;
        assert_eq!(out, "fresh");
        assert_eq!(fake.call(0).name, "codebase-memory-mcp__list_projects");
        assert_eq!(json_str(&fake.call(1).args, "project"), Some("grok"));
    }

    #[test]
    fn the_live_unknown_project_body_is_still_recognised() {
        assert!(super::is_unknown_project(
            super::OBSERVED_UNKNOWN_PROJECT_BODY
        ));
        assert!(super::is_unknown_project("Invalid project name: grok"));
        assert!(!super::is_unknown_project("no index is configured"));
    }
}
