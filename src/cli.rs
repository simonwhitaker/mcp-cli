use std::{collections::HashMap, path::PathBuf};

use anyhow::{Result, bail};
use clap::Parser;

use crate::transport::TransportConfig;

#[derive(Debug, Parser)]
#[command(
    name = "mcp",
    version,
    about = "Inspect and debug MCP servers from a terminal REPL"
)]
pub struct Cli {
    #[arg(long, help = "Print protocol and diagnostic detail")]
    pub debug: bool,

    #[arg(long, help = "Emit command output as JSON where possible")]
    pub json: bool,

    #[arg(long, value_name = "PATH", help = "Path to the REPL history file")]
    pub history: Option<PathBuf>,

    #[arg(long, help = "Disable ANSI colors")]
    pub no_color: bool,

    #[arg(
        long = "header",
        value_name = "NAME=VALUE",
        help = "HTTP header to send with every Streamable HTTP request"
    )]
    pub headers: Vec<String>,

    #[arg(
        long,
        value_name = "TOKEN",
        help = "Bearer token for Streamable HTTP authentication"
    )]
    pub bearer_token: Option<String>,

    #[arg(
        value_name = "TARGET",
        required = true,
        num_args = 1..,
        help = "HTTP(S) MCP endpoint URL, or command and arguments that expose a stdio MCP server"
    )]
    pub target: Vec<String>,
}

impl Cli {
    pub fn transport_config(&self) -> Result<TransportConfig> {
        let Some(first) = self.target.first() else {
            bail!("missing MCP server target");
        };

        if is_http_url(first) {
            if self.target.len() > 1 {
                bail!("extra positional arguments are not valid when TARGET is an HTTP URL");
            }
            Ok(TransportConfig::StreamableHttp {
                url: first.clone(),
                headers: parse_headers(&self.headers)?,
                bearer_token: self.bearer_token.clone(),
            })
        } else {
            if self.bearer_token.is_some() || !self.headers.is_empty() {
                bail!("--header and --bearer-token require an http:// or https:// TARGET");
            }
            TransportConfig::stdio(self.target.clone())
        }
    }
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn parse_headers(headers: &[String]) -> Result<HashMap<String, String>> {
    let mut parsed = HashMap::new();
    for header in headers {
        let Some((name, value)) = header.split_once('=') else {
            bail!("invalid --header value {header:?}; expected NAME=VALUE");
        };
        if name.trim().is_empty() {
            bail!("header name cannot be empty");
        }
        parsed.insert(name.trim().to_string(), value.trim().to_string());
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Cli;
    use crate::transport::TransportConfig;

    #[test]
    fn parses_stdio_transport() {
        let cli = Cli::parse_from(["mcp", "coral", "mcp-stdio"]);
        assert_eq!(
            cli.transport_config().unwrap(),
            TransportConfig::Stdio {
                command: "coral".to_string(),
                args: vec!["mcp-stdio".to_string()]
            }
        );
    }

    #[test]
    fn parses_http_transport_from_http_url_target() {
        let cli = Cli::parse_from([
            "mcp",
            "http://localhost:8000/mcp",
            "--header",
            "X-Test=yes",
            "--bearer-token",
            "secret",
        ]);
        assert_eq!(
            cli.transport_config().unwrap(),
            TransportConfig::StreamableHttp {
                url: "http://localhost:8000/mcp".to_string(),
                headers: [("X-Test".to_string(), "yes".to_string())].into(),
                bearer_token: Some("secret".to_string())
            }
        );
    }

    #[test]
    fn parses_http_transport_from_https_url_target() {
        let cli = Cli::parse_from(["mcp", "https://example.com/mcp"]);
        assert_eq!(
            cli.transport_config().unwrap(),
            TransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                headers: Default::default(),
                bearer_token: None
            }
        );
    }

    #[test]
    fn rejects_extra_args_after_http_url() {
        let cli = Cli::parse_from(["mcp", "http://localhost:8000/mcp", "extra"]);
        assert!(cli.transport_config().is_err());
    }

    #[test]
    fn rejects_http_options_for_stdio() {
        let cli = Cli::parse_from(["mcp", "--header", "X-Test=yes", "coral"]);
        assert!(cli.transport_config().is_err());
    }

    #[test]
    fn supports_stdio_command_args_after_separator() {
        let cli = Cli::parse_from(["mcp", "--", "server", "--server-flag"]);
        assert_eq!(
            cli.transport_config().unwrap(),
            TransportConfig::Stdio {
                command: "server".to_string(),
                args: vec!["--server-flag".to_string()]
            }
        );
    }
}
