use mcp_cli::{client_handler::InspectorClient, repl::parse_command, session::McpSession};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, Content, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::{MaybeSendFuture, RequestContext},
};
use serde_json::json;

#[derive(Debug, Clone)]
struct FakeServer;

impl FakeServer {
    fn new() -> Self {
        Self
    }
}

impl ServerHandler for FakeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("fake test server")
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + MaybeSendFuture + '_
    {
        std::future::ready(Ok(ListToolsResult {
            tools: vec![
                tool("echo", "Echo a message", vec![("message", "string")]),
                tool("fail", "Always fails", vec![]),
                tool(
                    "sum",
                    "Add two numbers",
                    vec![("a", "integer"), ("b", "integer")],
                ),
            ],
            ..Default::default()
        }))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = request.arguments.unwrap_or_default();
        match request.name.as_ref() {
            "echo" => {
                let message = args
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(message)]))
            }
            "sum" => {
                let a = args.get("a").and_then(|value| value.as_i64()).unwrap_or(0);
                let b = args.get("b").and_then(|value| value.as_i64()).unwrap_or(0);
                Ok(CallToolResult::success(vec![Content::text(
                    (a + b).to_string(),
                )]))
            }
            "fail" => Ok(CallToolResult::error(vec![Content::text(
                "intentional failure",
            )])),
            other => Err(McpError::invalid_request(
                format!("unknown tool: {other}"),
                Some(json!({"tool": other})),
            )),
        }
    }
}

fn tool(name: &str, description: &str, properties: Vec<(&str, &str)>) -> Tool {
    let mut schema_properties = serde_json::Map::new();
    let required = properties
        .iter()
        .map(|(name, _)| json!(name))
        .collect::<Vec<_>>();
    for (name, ty) in properties {
        schema_properties.insert(name.to_string(), json!({"type": ty}));
    }

    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), json!("object"));
    schema.insert("properties".to_string(), json!(schema_properties));
    schema.insert("required".to_string(), json!(required));
    Tool::new(name.to_string(), description.to_string(), schema)
}

#[tokio::test]
async fn session_initializes_lists_and_calls_tools() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server_task = tokio::spawn(async move {
        let server = FakeServer::new().serve(server_transport).await?;
        Ok::<_, anyhow::Error>(server.waiting().await?)
    });

    let handler = InspectorClient::new(true);
    let running = handler.clone().serve(client_transport).await?;
    let mut session = McpSession::from_running(running, handler).await?;

    assert!(session.server_info()?.instructions.as_deref() == Some("fake test server"));
    assert_eq!(session.tools().len(), 3);
    assert!(session.tool("echo").is_some());

    let echo = session
        .call_tool("echo", json!({"message": "hello from test"}))
        .await?;
    assert!(
        serde_json::to_value(echo)?
            .to_string()
            .contains("hello from test")
    );

    let sum = session.call_tool("sum", json!({"a": 2, "b": 40})).await?;
    assert!(serde_json::to_value(sum)?.to_string().contains("42"));

    let missing = session.call_tool("missing", json!({})).await;
    assert!(missing.is_err());

    session.refresh().await?;
    session.close().await?;
    let _ = server_task.await??;
    Ok(())
}

#[test]
fn malformed_json_is_a_friendly_parse_error() {
    let error = parse_command("tool echo --json '{bad json}'").unwrap_err();
    assert!(error.to_string().contains("key must be a string"));
}

#[test]
fn unknown_repl_command_is_reported() {
    let error = parse_command("wat").unwrap_err();
    assert!(error.to_string().contains("unknown command"));
}
