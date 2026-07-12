use mcp_cli::{client_handler::InspectorClient, session::McpSession, shell::parse_command};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        AnnotateAble, CallToolRequestParams, CallToolResult, Content, GetPromptRequestParams,
        GetPromptResult, ListPromptsResult, ListResourcesResult, ListToolsResult,
        PaginatedRequestParams, Prompt, PromptArgument, PromptMessage, PromptMessageContent,
        PromptMessageRole, RawResource, ReadResourceRequestParams, ReadResourceResult,
        ResourceContents, ServerCapabilities, ServerInfo, Tool,
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
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .build(),
        )
        .with_instructions("fake test server")
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + MaybeSendFuture + '_
    {
        std::future::ready(Ok(ListResourcesResult {
            resources: vec![
                RawResource::new("file:///single.txt", "single").no_annotation(),
                RawResource::new("file:///pair.txt", "pair").no_annotation(),
            ],
            ..Default::default()
        }))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        match request.uri.as_ref() {
            "file:///single.txt" => Ok(ReadResourceResult::new(vec![ResourceContents::text(
                "only content",
                "file:///single.txt",
            )])),
            // A read that returns more than one content item: the CLI used to
            // discard everything after the first.
            "file:///pair.txt" => Ok(ReadResourceResult::new(vec![
                ResourceContents::text("first content", "file:///pair.txt"),
                ResourceContents::text("second content", "file:///pair.txt"),
            ])),
            other => Err(McpError::resource_not_found(
                format!("unknown resource: {other}"),
                Some(json!({"uri": other})),
            )),
        }
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListPromptsResult, McpError>> + MaybeSendFuture + '_
    {
        let mut topic = PromptArgument::new("topic");
        topic.required = Some(true);

        std::future::ready(Ok(ListPromptsResult {
            prompts: vec![
                Prompt::new(
                    "greet",
                    Some("Greet someone about a topic"),
                    Some(vec![topic, PromptArgument::new("style")]),
                ),
                Prompt::new("empty", Some("Takes no arguments"), None),
            ],
            ..Default::default()
        }))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let args = request.arguments.unwrap_or_default();
        match request.name.as_ref() {
            "greet" => {
                let topic = args
                    .get("topic")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        McpError::invalid_params("missing required argument: topic", None)
                    })?;
                let style = args
                    .get("style")
                    .and_then(|value| value.as_str())
                    .unwrap_or("plain");
                let mut result = GetPromptResult::new(vec![PromptMessage::new(
                    PromptMessageRole::User,
                    PromptMessageContent::text(format!("Say hello about {topic} in {style}")),
                )]);
                result.description = Some("a greeting".to_string());
                Ok(result)
            }
            "empty" => Ok(GetPromptResult::new(vec![PromptMessage::new(
                PromptMessageRole::User,
                PromptMessageContent::text("no arguments here"),
            )])),
            other => Err(McpError::invalid_request(
                format!("unknown prompt: {other}"),
                Some(json!({"prompt": other})),
            )),
        }
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

#[tokio::test]
async fn session_lists_and_renders_prompts() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server_task = tokio::spawn(async move {
        let server = FakeServer::new().serve(server_transport).await?;
        Ok::<_, anyhow::Error>(server.waiting().await?)
    });

    let handler = InspectorClient::new(true);
    let running = handler.clone().serve(client_transport).await?;
    let mut session = McpSession::from_running(running, handler).await?;

    assert_eq!(session.prompts().len(), 2);

    let greet = session
        .get_prompt("greet", json!({"topic": "rust", "style": "haiku"}))
        .await?;
    assert_eq!(greet.description.as_deref(), Some("a greeting"));
    assert!(
        serde_json::to_value(&greet)?
            .to_string()
            .contains("Say hello about rust in haiku")
    );

    let empty = session.get_prompt("empty", json!({})).await?;
    assert_eq!(empty.messages.len(), 1);

    // The server, not the client, decides that a required argument is missing.
    let missing_argument = session.get_prompt("greet", json!({})).await;
    assert!(missing_argument.is_err());

    let missing_prompt = session.get_prompt("nope", json!({})).await;
    assert!(missing_prompt.is_err());

    session.close().await?;
    let _ = server_task.await??;
    Ok(())
}

#[tokio::test]
async fn session_reads_every_resource_content() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server_task = tokio::spawn(async move {
        let server = FakeServer::new().serve(server_transport).await?;
        Ok::<_, anyhow::Error>(server.waiting().await?)
    });

    let handler = InspectorClient::new(true);
    let running = handler.clone().serve(client_transport).await?;
    let mut session = McpSession::from_running(running, handler).await?;

    assert_eq!(session.resources().len(), 2);

    let single = session.get_resource("file:///single.txt").await?;
    assert_eq!(single.contents.len(), 1);

    let pair = session.get_resource("file:///pair.txt").await?;
    assert_eq!(pair.contents.len(), 2);
    let rendered = serde_json::to_value(&pair)?.to_string();
    assert!(rendered.contains("first content"));
    assert!(rendered.contains("second content"));

    let missing = session.get_resource("file:///nope.txt").await;
    assert!(missing.is_err());

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
fn unknown_shell_command_is_reported() {
    let error = parse_command("wat").unwrap_err();
    assert!(error.to_string().contains("unknown command"));
}
