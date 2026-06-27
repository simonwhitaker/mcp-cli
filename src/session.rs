use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rmcp::{
    RoleClient, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, ClientRequest, CustomRequest, ServerInfo,
        ServerResult, Tool,
    },
    service::{RunningService, ServiceError},
};
use serde_json::{Map, Value};

use crate::{
    client_handler::InspectorClient,
    transport::{TransportConfig, stdio_transport},
};

pub struct McpSession {
    running: RunningService<RoleClient, InspectorClient>,
    handler: InspectorClient,
    tools: Vec<Tool>,
}

impl McpSession {
    pub async fn connect(config: &TransportConfig, debug: bool) -> Result<Self> {
        let transport = stdio_transport(config)
            .with_context(|| format!("failed to spawn MCP server: {}", config.display_name()))?;
        let handler = InspectorClient::new(debug);
        let running = handler
            .clone()
            .serve(transport)
            .await
            .context("failed to initialize MCP server")?;
        let mut session = Self {
            running,
            handler,
            tools: Vec::new(),
        };
        session.refresh().await?;
        Ok(session)
    }

    pub async fn from_running(
        running: RunningService<RoleClient, InspectorClient>,
        handler: InspectorClient,
    ) -> Result<Self> {
        let mut session = Self {
            running,
            handler,
            tools: Vec::new(),
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

    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    pub async fn refresh(&mut self) -> Result<()> {
        self.tools = self
            .running
            .peer()
            .list_all_tools()
            .await
            .context("failed to list tools")?;
        self.tools.sort_by(|a, b| a.name.cmp(&b.name));
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

impl From<ServiceError> for crate::repl::ReplError {
    fn from(value: ServiceError) -> Self {
        Self::Command(value.to_string())
    }
}
