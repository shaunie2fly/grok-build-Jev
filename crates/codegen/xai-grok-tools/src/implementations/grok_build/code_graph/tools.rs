//! Model-facing codebase-memory tools. Args map onto the dispatch client.

use crate::types::output::ToolOutput;
use crate::types::tool::{ToolKind, ToolNamespace};

use super::client::{self, BlastRequest, SearchRequest, TraceRequest};
use super::limits::TRACE_DEPTH_DEFAULT;

const SEARCH_SYMBOLS_DESCRIPTION: &str = "Find a symbol in the codebase-memory graph for this workspace. Use when you know a function, type, or method name and not the file. Returns at most 15 rows. Pass name_pattern (regex) or query (words). Open the file with read_file before editing; graph lines drift. For a string, config, or text you can already search, use grep.";
const TRACE_CALLS_DESCRIPTION: &str = "Follow CALLS edges in the codebase-memory graph. function_name is the symbol name from search_symbols. direction is inbound (who calls it), outbound (what it calls), or both. depth is 1 to 3 and defaults to 2. Returns at most 30 rows and omits tests. For text that is not a call, use grep.";
const BLAST_RADIUS_DESCRIPTION: &str = "Map the git diff onto changed symbols and their callers in the codebase-memory graph. direction defaults to inbound. depth defaults to 2. Pass since, such as HEAD~3, to start from that ref. For a string inside the diff, use grep.";

fn default_depth() -> u32 {
    TRACE_DEPTH_DEFAULT
}

fn default_direction() -> TraceDirection {
    TraceDirection::Inbound
}

fn default_scope() -> BlastScope {
    BlastScope::Impact
}

fn is_blank(value: &Option<String>) -> bool {
    match value {
        Some(text) => text.trim().is_empty(),
        None => true,
    }
}

fn read_capabilities() -> xai_tool_protocol::ToolCapabilities {
    xai_tool_protocol::ToolCapabilities {
        is_read_only: true,
        tool_scope: Some(xai_tool_protocol::ToolScope::Read),
        ..Default::default()
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum TraceDirection {
    #[default]
    Inbound,
    Outbound,
    Both,
}

impl From<TraceDirection> for client::TraceDirection {
    fn from(direction: TraceDirection) -> Self {
        match direction {
            TraceDirection::Inbound => Self::Inbound,
            TraceDirection::Outbound => Self::Outbound,
            TraceDirection::Both => Self::Both,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum BlastScope {
    #[default]
    Impact,
    Files,
}

impl From<BlastScope> for client::BlastScope {
    fn from(scope: BlastScope) -> Self {
        match scope {
            BlastScope::Impact => Self::Impact,
            BlastScope::Files => Self::Files,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeGraphOutput(pub String);

impl xai_tool_runtime::ToolOutput for CodeGraphOutput {}

impl From<CodeGraphOutput> for ToolOutput {
    fn from(output: CodeGraphOutput) -> Self {
        ToolOutput::Text(output.0.into())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchSymbolsArgs {
    /// Regex matched against the symbol name.
    #[serde(default)]
    #[schemars(description = "Regex matched against the symbol name.")]
    pub name_pattern: Option<String>,
    /// Words matched against the symbol.
    #[serde(default)]
    #[schemars(description = "Words matched against the symbol.")]
    pub query: Option<String>,
    /// Optional symbol label, such as Function or Class.
    #[serde(default)]
    #[schemars(description = "Optional symbol label, such as Function or Class.")]
    pub label: Option<String>,
    /// Optional path filter.
    #[serde(default)]
    #[schemars(description = "Optional path filter.")]
    pub file_pattern: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct TraceCallsArgs {
    /// Symbol name from search_symbols.
    #[schemars(description = "Symbol name from search_symbols.")]
    pub function_name: String,
    /// inbound, outbound, or both. Defaults to inbound.
    #[serde(default)]
    #[schemars(
        description = "inbound (who calls it), outbound (what it calls), or both. Defaults to inbound.",
        default = "default_direction"
    )]
    pub direction: TraceDirection,
    /// 1 to 3. Defaults to 2.
    #[serde(default = "default_depth")]
    #[schemars(description = "1 to 3. Defaults to 2.", default = "default_depth")]
    pub depth: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BlastRadiusArgs {
    /// Git ref to start from, such as HEAD~3.
    #[serde(default)]
    #[schemars(description = "Git ref to start from, such as HEAD~3.")]
    pub since: Option<String>,
    /// impact or files. Defaults to impact.
    #[serde(default)]
    #[schemars(
        description = "impact or files. Defaults to impact.",
        default = "default_scope"
    )]
    pub scope: BlastScope,
    /// inbound, outbound, or both. Defaults to inbound.
    #[serde(default)]
    #[schemars(
        description = "inbound (who calls it), outbound (what it calls), or both. Defaults to inbound.",
        default = "default_direction"
    )]
    pub direction: TraceDirection,
    /// 1 to 3. Defaults to 2.
    #[serde(default = "default_depth")]
    #[schemars(description = "1 to 3. Defaults to 2.", default = "default_depth")]
    pub depth: u32,
}

#[derive(Debug, Default)]
pub struct SearchSymbolsTool;

impl crate::types::tool_metadata::ToolMetadata for SearchSymbolsTool {
    fn kind(&self) -> ToolKind {
        ToolKind::CodeGraph
    }

    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }

    fn description_template(&self) -> &str {
        SEARCH_SYMBOLS_DESCRIPTION
    }
}

impl xai_tool_runtime::Tool for SearchSymbolsTool {
    type Args = SearchSymbolsArgs;
    type Output = CodeGraphOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("search_symbols").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new(
            "search_symbols",
            crate::types::tool_metadata::ToolMetadata::sanitized_description_template(self),
        )
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        read_capabilities()
    }

    #[tracing::instrument(name = "tool.search_symbols", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: SearchSymbolsArgs,
    ) -> Result<CodeGraphOutput, xai_tool_runtime::ToolError> {
        if is_blank(&input.name_pattern) && is_blank(&input.query) {
            return Err(xai_tool_runtime::ToolError::invalid_arguments(
                "Pass name_pattern or query.",
            ));
        }
        let text = client::search_symbols(
            &ctx,
            SearchRequest {
                name_pattern: input.name_pattern,
                query: input.query,
                label: input.label,
                file_pattern: input.file_pattern,
            },
        )
        .await;
        Ok(CodeGraphOutput(text))
    }
}

#[cfg(test)]
impl SearchSymbolsTool {
    async fn run_for_test(
        args: SearchSymbolsArgs,
    ) -> Result<CodeGraphOutput, xai_tool_runtime::ToolError> {
        use crate::types::resources::Resources;
        use crate::types::tool_metadata::test_ctx;
        xai_tool_runtime::Tool::run(&Self, test_ctx(Resources::new().into_shared()), args).await
    }
}

#[derive(Debug, Default)]
pub struct TraceCallsTool;

impl crate::types::tool_metadata::ToolMetadata for TraceCallsTool {
    fn kind(&self) -> ToolKind {
        ToolKind::CodeGraph
    }

    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }

    fn description_template(&self) -> &str {
        TRACE_CALLS_DESCRIPTION
    }
}

impl xai_tool_runtime::Tool for TraceCallsTool {
    type Args = TraceCallsArgs;
    type Output = CodeGraphOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("trace_calls").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new(
            "trace_calls",
            crate::types::tool_metadata::ToolMetadata::sanitized_description_template(self),
        )
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        read_capabilities()
    }

    #[tracing::instrument(name = "tool.trace_calls", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: TraceCallsArgs,
    ) -> Result<CodeGraphOutput, xai_tool_runtime::ToolError> {
        let text = client::trace_calls(
            &ctx,
            TraceRequest {
                function_name: input.function_name,
                direction: input.direction.into(),
                depth: input.depth,
            },
        )
        .await;
        Ok(CodeGraphOutput(text))
    }
}

#[derive(Debug, Default)]
pub struct BlastRadiusTool;

impl crate::types::tool_metadata::ToolMetadata for BlastRadiusTool {
    fn kind(&self) -> ToolKind {
        ToolKind::CodeGraph
    }

    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }

    fn description_template(&self) -> &str {
        BLAST_RADIUS_DESCRIPTION
    }
}

impl xai_tool_runtime::Tool for BlastRadiusTool {
    type Args = BlastRadiusArgs;
    type Output = CodeGraphOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("blast_radius").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new(
            "blast_radius",
            crate::types::tool_metadata::ToolMetadata::sanitized_description_template(self),
        )
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        read_capabilities()
    }

    #[tracing::instrument(name = "tool.blast_radius", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: BlastRadiusArgs,
    ) -> Result<CodeGraphOutput, xai_tool_runtime::ToolError> {
        let text = client::blast_radius(
            &ctx,
            BlastRequest {
                since: input.since,
                scope: input.scope.into(),
                direction: input.direction.into(),
                depth: input.depth,
            },
        )
        .await;
        Ok(CodeGraphOutput(text))
    }
}

#[cfg(test)]
mod tests {
    use super::{BlastRadiusTool, SearchSymbolsArgs, SearchSymbolsTool, TraceCallsTool};
    use crate::types::tool_metadata::ToolMetadata;
    use xai_tool_runtime::ToolErrorKind;

    #[test]
    fn descriptions_stay_under_500_chars_and_name_grep() {
        for template in [
            SearchSymbolsTool.description_template(),
            TraceCallsTool.description_template(),
            BlastRadiusTool.description_template(),
        ] {
            assert!(template.chars().count() <= 500, "{template}");
            assert!(template.contains("grep"), "{template}");
        }
    }

    #[tokio::test]
    async fn search_symbols_without_a_pattern_is_an_argument_error() {
        let err = SearchSymbolsTool::run_for_test(SearchSymbolsArgs {
            name_pattern: None,
            query: Some("  ".into()),
            label: None,
            file_pattern: None,
        })
        .await
        .unwrap_err();
        assert_eq!(err.kind, ToolErrorKind::InvalidArguments);
        assert_eq!(err.detail, "Pass name_pattern or query.");
    }
}
