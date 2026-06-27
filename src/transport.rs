use std::collections::HashMap;

use anyhow::{Result, bail};
use rmcp::transport::TokioChildProcess;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportConfig {
    Stdio {
        command: String,
        args: Vec<String>,
    },
    StreamableHttp {
        url: String,
        headers: HashMap<String, String>,
    },
}

impl TransportConfig {
    pub fn stdio(command: Vec<String>) -> Result<Self> {
        let Some((program, args)) = command.split_first() else {
            bail!("missing MCP server command");
        };
        Ok(Self::Stdio {
            command: program.clone(),
            args: args.to_vec(),
        })
    }

    pub fn display_name(&self) -> String {
        match self {
            Self::Stdio { command, args } if args.is_empty() => command.clone(),
            Self::Stdio { command, args } => {
                let mut parts = Vec::with_capacity(args.len() + 1);
                parts.push(command.clone());
                parts.extend(args.iter().cloned());
                parts.join(" ")
            }
            Self::StreamableHttp { url, .. } => url.clone(),
        }
    }
}

pub fn stdio_transport(config: &TransportConfig) -> Result<TokioChildProcess> {
    match config {
        TransportConfig::Stdio { command, args } => {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args);
            Ok(TokioChildProcess::new(cmd)?)
        }
        TransportConfig::StreamableHttp { .. } => {
            bail!("Streamable HTTP transport is planned but not implemented yet")
        }
    }
}
