use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rmcp::{
    RoleClient, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, ClientRequest, CustomRequest, Prompt,
        ReadResourceRequestParams, Resource, ResourceContents, ServerInfo, ServerResult, Tool,
    },
    service::{RunningService, ServiceError},
};
use serde_json::{Map, Value};

use crate::{
    client_handler::InspectorClient,
    transport::{TransportConfig, stdio_transport, streamable_http_transport},
};

pub struct McpSession {
    running: RunningService<RoleClient, InspectorClient>,
    handler: InspectorClient,
    tools: Vec<Tool>,
    resources: Vec<Resource>,
    prompts: Vec<Prompt>,
}

impl McpSession {
    pub async fn connect(config: &TransportConfig, debug: bool) -> Result<Self> {
        let handler = InspectorClient::new(debug);

        match config {
            TransportConfig::Stdio { command, args } => {
                let transport = stdio_transport(command, args).with_context(|| {
                    format!("failed to spawn MCP server: {}", config.display_name())
                })?;
                let running = handler
                    .clone()
                    .serve(transport)
                    .await
                    .context("failed to initialize MCP stdio server")?;
                Self::from_running(running, handler).await
            }
            TransportConfig::StreamableHttp {
                url,
                headers,
                bearer_token,
            } => {
                let transport = streamable_http_transport(url, headers, bearer_token.as_deref())
                    .with_context(|| {
                        format!("failed to configure MCP Streamable HTTP transport: {url}")
                    })?;
                let running = handler
                    .clone()
                    .serve(transport)
                    .await
                    .context("failed to initialize MCP Streamable HTTP server")?;
                Self::from_running(running, handler).await
            }
        }
    }

    pub async fn from_running(
        running: RunningService<RoleClient, InspectorClient>,
        handler: InspectorClient,
    ) -> Result<Self> {
        let mut session = Self {
            running,
            handler,
            tools: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
        };
        session.refresh().await?;
        Ok(session)
    }

    pub fn server_info(&self) -> Result<Arc<ServerInfo>> {
        self.running
            .peer()
            .peer_info()
            .context("server did not provide initialize info")
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }

    pub fn prompts(&self) -> &[Prompt] {
        &self.prompts
    }

    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    pub async fn refresh(&mut self) -> Result<()> {
        let server_info = self.server_info()?;

        self.tools = if server_info.capabilities.tools.is_some() {
            self.running
                .peer()
                .list_all_tools()
                .await
                .context("failed to list tools")?
        } else {
            Vec::new()
        };
        self.tools.sort_by(|a, b| a.name.cmp(&b.name));

        self.resources = if server_info.capabilities.resources.is_some() {
            self.running
                .peer()
                .list_all_resources()
                .await
                .context("failed to list resources")?
        } else {
            Vec::new()
        };
        self.resources.sort_by(|a, b| a.uri.cmp(&b.uri));

        self.prompts = if server_info.capabilities.prompts.is_some() {
            self.running
                .peer()
                .list_all_prompts()
                .await
                .context("failed to list prompts")?
        } else {
            Vec::new()
        };
        self.prompts.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(())
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<CallToolResult> {
        let arguments = match arguments {
            Value::Object(map) if map.is_empty() => None,
            Value::Object(map) => Some(map),
            other => bail!("tool arguments must be a JSON object, got {other}"),
        };

        self.running
            .peer()
            .call_tool(match arguments {
                Some(arguments) => {
                    CallToolRequestParams::new(name.to_string()).with_arguments(arguments)
                }
                None => CallToolRequestParams::new(name.to_string()),
            })
            .await
            .with_context(|| format!("tool call failed: {name}"))
    }

    pub async fn get_resource(&self, uri: &str) -> Result<ResourceContents> {
        let result = self
            .running
            .peer()
            .read_resource(ReadResourceRequestParams::new(uri.to_string()))
            .await
            .context(format!("failed to load resource: {uri}"))?;
        Ok(result
            .contents
            .first()
            .context(format!("resource {uri} has no contents"))?
            .clone())
    }

    pub async fn get_prompt(&self, name: &str) -> Result<Prompt> {
        self.prompts
            .iter()
            .find(|prompt| prompt.name == name)
            .cloned()
            .with_context(|| format!("prompt not found: {name}"))
    }

    pub async fn raw_request(&self, method: String, params: Option<Value>) -> Result<Value> {
        let result = self
            .running
            .peer()
            .send_request(ClientRequest::CustomRequest(CustomRequest::new(
                method, params,
            )))
            .await
            .context("raw MCP request failed")?;

        Ok(match result {
            ServerResult::CustomResult(result) => result.0,
            other => serde_json::to_value(other)?,
        })
    }

    pub async fn drain_notifications(&self) -> Vec<crate::client_handler::ClientNotification> {
        self.handler.drain_notifications().await
    }

    pub async fn close(&mut self) -> Result<()> {
        self.running
            .close()
            .await
            .context("failed to close MCP session")?;
        Ok(())
    }
}

pub fn object_from_value(value: Value) -> Result<Map<String, Value>> {
    match value {
        Value::Object(map) => Ok(map),
        other => bail!("expected a JSON object, got {other}"),
    }
}

impl From<ServiceError> for crate::shell::ShellError {
    fn from(value: ServiceError) -> Self {
        Self::Command(value.to_string())
    }
}
