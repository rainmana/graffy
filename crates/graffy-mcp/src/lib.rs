//! MCP integration (docs/design/phase-2-mcp.md).
//!
//! * [`LiveServer`] — an rmcp stdio client: connect, discover (tools +
//!   annotations), call. Streamable-HTTP lands in the next slice; the
//!   registry already stores the transport so specs never change.
//! * [`RegistryToolInvoker`] — the [`ToolInvoker`] implementation the
//!   executor uses: connected servers by logical name.
//! * [`generate_facade`] — turns a discovered tool into a durable facade
//!   graph (`prepare → [approve] → invoke → digest → respond`), with the
//!   role seeded from the server's own annotations (`readOnlyHint` →
//!   evidence; `destructiveHint` or unknown → effector + approval gate).
//!
//! Skills front the endpoint: the prepare node's system knowledge carries the
//! tool's schema and any usage knowledge; a bare tool call never reaches a
//! graph. MCP prompts-primitive import arrives with the next slice.

use std::collections::HashMap;

use thiserror::Error;
use tokio::process::Command;

use graffy_core::error::ToolError;
use graffy_core::exec::{ToolInvoker, ToolResponse};
use graffy_core::spec::{EdgeSpec, GraphMeta, GraphSpec, NodeSpec, PolicySpec};

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, GetPromptRequestParams};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};

pub mod interview;

/// MCP-plane errors.
#[derive(Debug, Error)]
pub enum McpError {
    #[error("failed to spawn MCP server process: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("MCP handshake failed: {0}")]
    Handshake(String),
    #[error("MCP service error: {0}")]
    Service(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// One tool as discovered from a live server.
#[derive(Debug, Clone)]
pub struct DiscoveredTool {
    pub name: String,
    pub description: String,
    pub read_only: Option<bool>,
    pub destructive: Option<bool>,
    /// The tool's JSON input schema, verbatim.
    pub schema_json: String,
}

/// A server-shipped prompt — the author's own usage knowledge.
#[derive(Debug, Clone)]
pub struct DiscoveredPrompt {
    pub name: String,
    pub description: String,
    /// Rendered text, when the prompt takes no required arguments.
    pub content: Option<String>,
}

/// Discovery snapshot for a server.
#[derive(Debug, Clone)]
pub struct Discovery {
    pub tools: Vec<DiscoveredTool>,
    pub prompts: Vec<DiscoveredPrompt>,
}

/// Fold server-shipped prompts into prepare-node usage knowledge
/// (design doc §2: skills front the endpoint). Bounded so a chatty server
/// cannot flood every facade's system content.
pub fn usage_knowledge_from_prompts(prompts: &[DiscoveredPrompt]) -> Option<String> {
    const CAP: usize = 4000;
    let mut sections = Vec::new();
    for prompt in prompts {
        let mut section = format!("## {}\n{}", prompt.name, prompt.description);
        if let Some(content) = &prompt.content {
            section.push('\n');
            section.push_str(content);
        }
        sections.push(section);
    }
    if sections.is_empty() {
        return None;
    }
    let mut joined = sections.join("\n\n");
    if joined.chars().count() > CAP {
        joined = joined.chars().take(CAP).collect::<String>()
            + "\n… (truncated — full prompts in the server registry)";
    }
    Some(joined)
}

/// A connected stdio MCP server.
pub struct LiveServer {
    service: rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
}

impl LiveServer {
    /// Spawn and handshake a stdio server (`command args…`).
    pub async fn connect_stdio(command: &str, args: &[String]) -> Result<Self, McpError> {
        let transport = TokioChildProcess::new(Command::new(command).configure(|cmd| {
            for arg in args {
                cmd.arg(arg);
            }
        }))?;
        let service = ().serve(transport).await.map_err(|e| McpError::Handshake(e.to_string()))?;
        Ok(Self { service })
    }

    /// List every tool with its annotations.
    pub async fn discover(&self) -> Result<Discovery, McpError> {
        let tools = self
            .service
            .list_all_tools()
            .await
            .map_err(|e| McpError::Service(e.to_string()))?;
        let mut discovered = Vec::new();
        for tool in tools {
            let (read_only, destructive) = tool
                .annotations
                .as_ref()
                .map(|a| (a.read_only_hint, a.destructive_hint))
                .unwrap_or((None, None));
            discovered.push(DiscoveredTool {
                name: tool.name.to_string(),
                description: tool.description.as_deref().unwrap_or_default().to_string(),
                read_only,
                destructive,
                schema_json: serde_json::to_string(&*tool.input_schema)?,
            });
        }
        let mut prompts = Vec::new();
        match self.service.list_all_prompts().await {
            Ok(listed) => {
                for prompt in listed {
                    let has_required_args = prompt
                        .arguments
                        .as_ref()
                        .is_some_and(|args| args.iter().any(|a| a.required.unwrap_or(false)));
                    let mut discovered_prompt = DiscoveredPrompt {
                        name: prompt.name.clone(),
                        description: prompt.description.clone().unwrap_or_default(),
                        content: None,
                    };
                    if !has_required_args {
                        match self
                            .service
                            .get_prompt_once(GetPromptRequestParams::new(prompt.name.clone()))
                            .await
                        {
                            Ok(rmcp::model::GetPromptResponse::Complete(result)) => {
                                let mut parts = Vec::new();
                                for message in &result.messages {
                                    if let Some(text) = message.content.as_text() {
                                        parts.push(text.text.clone());
                                    }
                                }
                                if !parts.is_empty() {
                                    discovered_prompt.content = Some(parts.join("\n"));
                                }
                            }
                            Ok(other) => {
                                tracing::debug!(prompt = %prompt.name, ?other, "unsupported prompt response kind");
                            }
                            Err(err) => {
                                tracing::warn!(prompt = %prompt.name, %err, "prompt fetch failed");
                            }
                        }
                    }
                    prompts.push(discovered_prompt);
                }
            }
            Err(err) => {
                // Servers without the prompts capability commonly error here;
                // that is absence of knowledge, not a failure.
                tracing::debug!(%err, "prompt listing unavailable");
            }
        }

        Ok(Discovery {
            tools: discovered,
            prompts,
        })
    }

    /// Call a tool; returns (concatenated text, is_error).
    pub async fn call(
        &self,
        tool: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(String, bool), McpError> {
        let response = self
            .service
            .call_tool_once(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments))
            .await
            .map_err(|e| McpError::Service(e.to_string()))?;
        let result = match response {
            rmcp::model::CallToolResponse::Complete(result) => result,
            other => {
                return Err(McpError::Service(format!(
                    "server returned a response kind graffy does not support yet ({other:?}) — \
                     input-required elicitation and tasks map onto the approval machinery in a \
                     later slice"
                )));
            }
        };

        let mut parts: Vec<String> = Vec::new();
        for content in &result.content {
            if let Some(text) = content.as_text() {
                parts.push(text.text.clone());
            }
        }
        if parts.is_empty()
            && let Some(structured) = &result.structured_content
        {
            parts.push(serde_json::to_string(structured)?);
        }
        Ok((parts.join("\n"), result.is_error.unwrap_or(false)))
    }

    /// Cancel the service and reap the child.
    pub async fn shutdown(self) {
        let _ = self.service.cancel().await;
    }
}

// ---------------------------------------------------------------------------
// The executor-facing tool plane
// ---------------------------------------------------------------------------

/// Transport binding for one registered server (from the store; specs only
/// ever name servers logically — §5 of the design doc).
#[derive(Debug, Clone)]
pub struct ServerBinding {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// [`ToolInvoker`] over a set of connected servers.
pub struct RegistryToolInvoker {
    live: HashMap<String, LiveServer>,
}

impl RegistryToolInvoker {
    /// Connect every binding up front (few servers, cheap handshakes; lazy
    /// connection is a later optimization, not a semantics change).
    pub async fn connect_all(bindings: Vec<ServerBinding>) -> Result<Self, McpError> {
        let mut live = HashMap::new();
        for binding in bindings {
            tracing::info!(server = %binding.name, command = %binding.command, "connecting MCP server");
            let server = LiveServer::connect_stdio(&binding.command, &binding.args).await?;
            live.insert(binding.name, server);
        }
        Ok(Self { live })
    }

    pub fn server_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.live.keys().cloned().collect();
        names.sort();
        names
    }

    /// Shut every server down cleanly.
    pub async fn shutdown(self) {
        for (_, server) in self.live {
            server.shutdown().await;
        }
    }
}

#[async_trait::async_trait]
impl ToolInvoker for RegistryToolInvoker {
    async fn invoke(
        &self,
        server: &str,
        tool: &str,
        args_json: &str,
    ) -> Result<ToolResponse, ToolError> {
        let Some(live) = self.live.get(server) else {
            return Err(ToolError::Unavailable(format!(
                "MCP server '{server}' is not connected"
            )));
        };
        // Unparseable arguments are wrapped, never guessed at (ADR-0005
        // discipline): the server sees {"input": <raw>} and can reject it.
        let arguments = match serde_json::from_str::<serde_json::Value>(args_json) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => {
                let mut map = serde_json::Map::new();
                map.insert(
                    "input".to_owned(),
                    serde_json::Value::String(args_json.to_owned()),
                );
                map
            }
        };
        let started = std::time::Instant::now();
        let (text, is_error) = live
            .call(tool, arguments)
            .await
            .map_err(|e| ToolError::Call(e.to_string()))?;
        Ok(ToolResponse {
            text,
            is_error,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

// ---------------------------------------------------------------------------
// Role seeding + facade generation
// ---------------------------------------------------------------------------

/// Seed a tool's role from its annotations (design doc §3): read-only tools
/// are evidence; destructive or unannotated tools are effectors — nothing
/// destructive ever slips in as "just research."
pub fn seed_role(tool: &DiscoveredTool, server_default: &str) -> &'static str {
    if tool.destructive == Some(true) {
        return "effector";
    }
    if tool.read_only == Some(true) {
        return "evidence";
    }
    match server_default {
        "evidence" => "evidence",
        _ => "effector",
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn param(value: &str) -> toml::Value {
    toml::Value::String(value.to_owned())
}

/// Generate the facade graph for one discovered tool
/// (`prepare → [approve] → invoke → digest → respond`, per §2).
pub fn generate_facade(
    server: &str,
    tool: &DiscoveredTool,
    role: &str,
    evidence_level: &str,
    usage_knowledge: Option<&str>,
) -> GraphSpec {
    let gated = role == "effector";
    let graph_id = format!("graffy.mcp.{}.{}", sanitize(server), sanitize(&tool.name));

    let mut prepare_system = format!(
        "You are the prepare node fronting the MCP tool '{}/{}'. Tool description: {}. \
         From the goal and context, emit ONLY a JSON object of arguments conforming to \
         this JSON schema (no prose, no code fences):\n{}",
        server, tool.name, tool.description, tool.schema_json
    );
    if let Some(knowledge) = usage_knowledge {
        prepare_system.push_str("\n\nUsage knowledge for this server:\n");
        prepare_system.push_str(knowledge);
    }

    let mut prepare_params = toml::Table::new();
    prepare_params.insert("iu_role".into(), param("tool-args"));
    prepare_params.insert("system".into(), param(&prepare_system));

    let mut invoke_params = toml::Table::new();
    invoke_params.insert("server".into(), param(server));
    invoke_params.insert("tool".into(), param(&tool.name));
    invoke_params.insert("evidence_level".into(), param(evidence_level));

    let mut digest_params = toml::Table::new();
    digest_params.insert("iu_role".into(), param("draft"));
    digest_params.insert("context_roles".into(), param("tool-result"));
    digest_params.insert(
        "system".into(),
        param(&format!(
            "Turn the tool result into a faithful, grounded answer to the goal. Preserve \
             every distinction the goal depends on; state plainly that the data came from \
             the MCP tool '{}/{}'. Do not add claims the result does not support.",
            server, tool.name
        )),
    );

    let mut approve_params = toml::Table::new();
    approve_params.insert(
        "question".into(),
        param(&format!(
            "Allow the MCP tool '{}/{}' to run with the prepared arguments?",
            server, tool.name
        )),
    );

    let mut nodes = vec![
        NodeSpec {
            id: "intake".into(),
            kind: "intake".into(),
            description: "Decompose the request into Information Units.".into(),
            model_tier: None,
            params: toml::Table::new(),
        },
        NodeSpec {
            id: "prepare".into(),
            kind: "model".into(),
            description: format!("Skill-fronted argument construction for {}.", tool.name),
            model_tier: Some("fast".into()),
            params: prepare_params,
        },
    ];
    if gated {
        nodes.push(NodeSpec {
            id: "approve".into(),
            kind: "approval".into(),
            description: "Effector gate: a human releases the call (design doc §3/§6).".into(),
            model_tier: None,
            params: approve_params,
        });
    }
    nodes.push(NodeSpec {
        id: "invoke".into(),
        kind: "tool.invoke".into(),
        description: "Transport call; result lands as hash-addressed MCP evidence.".into(),
        model_tier: None,
        params: invoke_params,
    });
    nodes.push(NodeSpec {
        id: "digest".into(),
        kind: "model".into(),
        description: "Grounded digestion of the tool result into IUs.".into(),
        model_tier: Some("fast".into()),
        params: digest_params,
    });
    nodes.push(NodeSpec {
        id: "respond".into(),
        kind: "respond".into(),
        description: "Deliver the grounded answer.".into(),
        model_tier: None,
        params: toml::Table::new(),
    });

    let mut edges = vec![EdgeSpec {
        from: "intake".into(),
        to: "prepare".into(),
        when: None,
    }];
    if gated {
        edges.push(EdgeSpec {
            from: "prepare".into(),
            to: "approve".into(),
            when: None,
        });
        edges.push(EdgeSpec {
            from: "approve".into(),
            to: "invoke".into(),
            when: Some("approval == 'approved'".into()),
        });
    } else {
        edges.push(EdgeSpec {
            from: "prepare".into(),
            to: "invoke".into(),
            when: None,
        });
    }
    edges.push(EdgeSpec {
        from: "invoke".into(),
        to: "digest".into(),
        when: Some("tool.ok == 'true'".into()),
    });
    edges.push(EdgeSpec {
        from: "digest".into(),
        to: "respond".into(),
        when: None,
    });

    GraphSpec {
        graph: GraphMeta {
            id: graph_id,
            name: format!("MCP facade: {}/{}", server, tool.name),
            version: "0.1.0".into(),
            description: format!(
                "Skill-fronted facade for MCP tool '{}' on server '{}' (role: {role}).",
                tool.name, server
            ),
            license: Some("GPL-3.0-or-later".into()),
            authors: vec![format!("generated by graffy mcp add {server}")],
            tags: vec![
                "mcp".into(),
                "facade".into(),
                sanitize(server),
                role.to_owned(),
            ],
        },
        nodes,
        edges,
        policy: PolicySpec::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graffy_core::graph::CompiledGraph;

    fn tool(read_only: Option<bool>, destructive: Option<bool>) -> DiscoveredTool {
        DiscoveredTool {
            name: "Echo Tool".into(),
            description: "echoes".into(),
            read_only,
            destructive,
            schema_json: r#"{"type":"object","properties":{"message":{"type":"string"}}}"#.into(),
        }
    }

    #[test]
    fn roles_seed_conservatively_from_annotations() {
        assert_eq!(seed_role(&tool(Some(true), None), "effector"), "evidence");
        assert_eq!(seed_role(&tool(None, Some(true)), "evidence"), "effector");
        assert_eq!(
            seed_role(&tool(Some(true), Some(true)), "evidence"),
            "effector"
        );
        assert_eq!(seed_role(&tool(None, None), "evidence"), "evidence");
        assert_eq!(seed_role(&tool(None, None), "effector"), "effector");
        assert_eq!(seed_role(&tool(None, None), "unknown"), "effector");
    }

    #[test]
    fn evidence_facade_compiles_without_a_gate() {
        let spec = generate_facade("srv", &tool(Some(true), None), "evidence", "L2", None);
        assert_eq!(spec.graph.id, "graffy.mcp.srv.echo-tool");
        assert!(!spec.nodes.iter().any(|n| n.kind == "approval"));
        let toml_text = spec.to_toml_string().unwrap();
        let reparsed = GraphSpec::from_toml_str(&toml_text).unwrap();
        CompiledGraph::compile(&reparsed).expect("evidence facade must compile");
    }

    #[test]
    fn effector_facade_compiles_with_an_approval_gate() {
        let spec = generate_facade(
            "srv",
            &tool(None, Some(true)),
            "effector",
            "L1",
            Some("only use during business hours"),
        );
        assert!(spec.nodes.iter().any(|n| n.kind == "approval"));
        let prepare = spec.nodes.iter().find(|n| n.id == "prepare").unwrap();
        let system = prepare
            .params
            .get("system")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(
            system.contains("business hours"),
            "usage knowledge fronts the endpoint"
        );
        let toml_text = spec.to_toml_string().unwrap();
        let reparsed = GraphSpec::from_toml_str(&toml_text).unwrap();
        CompiledGraph::compile(&reparsed).expect("effector facade must compile");
    }

    /// Hermetic full-protocol round-trip against the committed Python
    /// fixture server — genuine JSON-RPC over genuine stdio, runs in CI
    /// with zero network (python3 is present on all CI runners).
    #[tokio::test]
    async fn fixture_server_full_roundtrip_over_real_stdio() {
        let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixture/mini_server.py");
        let server = LiveServer::connect_stdio("python3", &[fixture.to_owned()])
            .await
            .expect("spawn + handshake the fixture server");

        let discovery = server.discover().await.expect("discovery");
        assert_eq!(discovery.tools.len(), 1);
        let echo = &discovery.tools[0];
        assert_eq!(echo.name, "echo");
        assert_eq!(echo.read_only, Some(true));
        assert_eq!(echo.destructive, Some(false));
        assert_eq!(seed_role(echo, "effector"), "evidence");
        assert!(echo.schema_json.contains("message"));

        assert_eq!(discovery.prompts.len(), 1, "fixture ships one usage prompt");
        let usage = &discovery.prompts[0];
        assert_eq!(usage.name, "usage");
        let content = usage
            .content
            .as_deref()
            .expect("no-arg prompt content fetched");
        assert!(content.contains("echo tool expects"));
        let knowledge =
            usage_knowledge_from_prompts(&discovery.prompts).expect("knowledge folds from prompts");
        assert!(knowledge.contains("## usage"));
        assert!(knowledge.contains("keep messages short"));

        let mut args = serde_json::Map::new();
        args.insert(
            "message".into(),
            serde_json::Value::String("round trip".into()),
        );
        let (text, is_error) = server.call("echo", args).await.expect("echo call");
        assert!(!is_error);
        assert_eq!(text, "fixture-echo: round trip");
        server.shutdown().await;
    }

    /// Real-server integration: requires network + npx.
    /// Run manually: cargo test -p graffy-mcp -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires network + npx (real MCP server)"]
    async fn everything_server_discover_and_call_roundtrip() {
        let server = LiveServer::connect_stdio(
            "npx",
            &[
                "-y".into(),
                "@modelcontextprotocol/server-everything".into(),
            ],
        )
        .await
        .expect("connect to server-everything via npx/stdio");

        let discovery = server.discover().await.expect("discovery");
        assert!(
            discovery.tools.iter().any(|t| t.name == "echo"),
            "expected an echo tool, got: {:?}",
            discovery
                .tools
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
        );

        let mut args = serde_json::Map::new();
        args.insert(
            "message".into(),
            serde_json::Value::String("hello from graffy".into()),
        );
        let (text, is_error) = server.call("echo", args).await.expect("echo call");
        assert!(!is_error);
        assert!(text.contains("hello from graffy"), "got: {text}");
        server.shutdown().await;
    }
}
