use std::{future::Future, sync::Arc};

use rmcp::{
    ClientHandler, ErrorData as McpError, RoleClient,
    model::{
        CancelledNotificationParam, ClientCapabilities, ClientInfo, CreateElicitationRequestParams,
        CreateElicitationResult, ElicitationAction, Implementation,
        LoggingMessageNotificationParam, ProgressNotificationParam,
    },
    service::{MaybeSendFuture, NotificationContext, RequestContext},
};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct InspectorClient {
    notifications: Arc<Mutex<Vec<ClientNotification>>>,
    debug: bool,
}

#[derive(Debug, Clone)]
pub enum ClientNotification {
    Log(LoggingMessageNotificationParam),
    Progress(ProgressNotificationParam),
    Cancelled(CancelledNotificationParam),
    ElicitationDeclined(String),
}

impl InspectorClient {
    pub fn new(debug: bool) -> Self {
        Self {
            notifications: Arc::new(Mutex::new(Vec::new())),
            debug,
        }
    }

    pub async fn drain_notifications(&self) -> Vec<ClientNotification> {
        let mut notifications = self.notifications.lock().await;
        notifications.drain(..).collect()
    }

    async fn push_notification(&self, notification: ClientNotification) {
        self.notifications.lock().await.push(notification);
    }
}

impl ClientHandler for InspectorClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        let mut implementation = Implementation::new("mcp-cli", env!("CARGO_PKG_VERSION"));
        implementation.title = Some("MCP Inspector CLI".into());
        info.client_info = implementation;
        info.capabilities = ClientCapabilities::default();
        info
    }

    fn create_elicitation(
        &self,
        request: CreateElicitationRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> impl Future<Output = Result<CreateElicitationResult, McpError>> + MaybeSendFuture + '_
    {
        let debug = self.debug;
        async move {
            if debug {
                let _ = request;
            }
            Ok(CreateElicitationResult {
                action: ElicitationAction::Decline,
                content: None,
                meta: None,
            })
        }
    }

    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.push_notification(ClientNotification::Log(params))
            .await;
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.push_notification(ClientNotification::Progress(params))
            .await;
    }

    async fn on_cancelled(
        &self,
        params: CancelledNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.push_notification(ClientNotification::Cancelled(params))
            .await;
    }
}
