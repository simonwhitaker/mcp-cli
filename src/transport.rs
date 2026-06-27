use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use http::{HeaderName, HeaderValue};
use rmcp::transport::{
    StreamableHttpClientTransport, TokioChildProcess,
    streamable_http_client::StreamableHttpClientTransportConfig,
};
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
        bearer_token: Option<String>,
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

pub fn stdio_transport(command: &str, args: &[String]) -> Result<TokioChildProcess> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    Ok(TokioChildProcess::new(cmd)?)
}

pub fn streamable_http_transport(
    url: &str,
    headers: &HashMap<String, String>,
    bearer_token: Option<&str>,
) -> Result<StreamableHttpClientTransport<reqwest::Client>> {
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
    if let Some(token) = bearer_token {
        config = config.auth_header(token.to_string());
    }
    config = config.custom_headers(parse_http_headers(headers)?);
    Ok(StreamableHttpClientTransport::from_config(config))
}

fn parse_http_headers(
    headers: &HashMap<String, String>,
) -> Result<HashMap<HeaderName, HeaderValue>> {
    let mut parsed = HashMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .with_context(|| format!("invalid HTTP header name: {name}"))?;
        let value = HeaderValue::from_str(value)
            .with_context(|| format!("invalid value for HTTP header: {name}"))?;
        parsed.insert(name, value);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{TransportConfig, parse_http_headers};

    #[test]
    fn parses_http_headers() {
        let headers = [("X-Test".to_string(), "yes".to_string())].into();
        let parsed = parse_http_headers(&headers).unwrap();
        assert_eq!(parsed[&http::HeaderName::from_static("x-test")], "yes");
    }

    #[test]
    fn display_name_uses_http_url() {
        let config = TransportConfig::StreamableHttp {
            url: "http://localhost:8000/mcp".to_string(),
            headers: Default::default(),
            bearer_token: None,
        };
        assert_eq!(config.display_name(), "http://localhost:8000/mcp");
    }
}
