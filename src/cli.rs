use std::{collections::HashMap, env, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;

use crate::transport::TransportConfig;

#[derive(Debug, Parser)]
#[command(
    name = "mcp",
    version,
    about = "Inspect and debug MCP servers from an interactive shell"
)]
pub struct Cli {
    #[arg(long, help = "Print protocol and diagnostic detail")]
    pub debug: bool,

    #[arg(long, help = "Emit command output as JSON where possible")]
    pub json: bool,

    #[arg(long, value_name = "PATH", help = "Path to the shell history file")]
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
        value_name = "VAR",
        help = "Read a Streamable HTTP bearer token from an environment variable"
    )]
    pub bearer_token_env: Option<String>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Read a Streamable HTTP bearer token from a file"
    )]
    pub bearer_token_file: Option<PathBuf>,

    #[arg(
        long,
        help = "Prompt for a Streamable HTTP bearer token without echoing input"
    )]
    pub bearer_token_prompt: bool,

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
        self.transport_config_with_resolvers(|name| env::var(name), prompt_for_bearer_token)
    }

    fn transport_config_with_resolvers(
        &self,
        env_var: impl Fn(&str) -> std::result::Result<String, env::VarError>,
        prompt: impl Fn() -> Result<String>,
    ) -> Result<TransportConfig> {
        let Some(first) = self.target.first() else {
            bail!("missing MCP server target");
        };

        let token_source = self.bearer_token_source()?;

        if is_http_url(first) {
            if self.target.len() > 1 {
                bail!("extra positional arguments are not valid when TARGET is an HTTP URL");
            }
            Ok(TransportConfig::StreamableHttp {
                url: first.clone(),
                headers: parse_headers(&self.headers)?,
                bearer_token: resolve_bearer_token(token_source, env_var, prompt)?,
            })
        } else {
            if token_source.is_some() || !self.headers.is_empty() {
                bail!("--header and bearer token options require an http:// or https:// TARGET");
            }
            TransportConfig::stdio(self.target.clone())
        }
    }

    fn bearer_token_source(&self) -> Result<Option<BearerTokenSource<'_>>> {
        let mut sources = Vec::new();
        if let Some(var) = &self.bearer_token_env {
            sources.push(BearerTokenSource::Env(var.as_str()));
        }
        if let Some(path) = &self.bearer_token_file {
            sources.push(BearerTokenSource::File(path));
        }
        if self.bearer_token_prompt {
            sources.push(BearerTokenSource::Prompt);
        }

        match sources.len() {
            0 => Ok(None),
            1 => Ok(sources.into_iter().next()),
            _ => bail!(
                "provide only one bearer token source: --bearer-token-env, --bearer-token-file, or --bearer-token-prompt"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum BearerTokenSource<'a> {
    Env(&'a str),
    File(&'a PathBuf),
    Prompt,
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn resolve_bearer_token(
    source: Option<BearerTokenSource<'_>>,
    env_var: impl Fn(&str) -> std::result::Result<String, env::VarError>,
    prompt: impl Fn() -> Result<String>,
) -> Result<Option<String>> {
    let Some(source) = source else {
        return Ok(None);
    };

    let token = match source {
        BearerTokenSource::Env(name) => env_var(name)
            .with_context(|| format!("failed to read bearer token environment variable {name}"))?,
        BearerTokenSource::File(path) => fs::read_to_string(path)
            .with_context(|| format!("failed to read bearer token file: {}", path.display()))?,
        BearerTokenSource::Prompt => prompt()?,
    };

    let token = token.trim().to_string();
    if token.is_empty() {
        bail!("bearer token source resolved to an empty token");
    }
    Ok(Some(token))
}

fn prompt_for_bearer_token() -> Result<String> {
    rpassword::prompt_password("Bearer token: ").context("failed to read bearer token")
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

    fn no_prompt() -> anyhow::Result<String> {
        panic!("prompt should not be called in this test")
    }

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
            "--bearer-token-env",
            "MCP_TEST_TOKEN",
        ]);
        assert_eq!(
            cli.transport_config_with_resolvers(
                |name| match name {
                    "MCP_TEST_TOKEN" => Ok("secret".to_string()),
                    _ => Err(std::env::VarError::NotPresent),
                },
                no_prompt
            )
            .unwrap(),
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
    fn reads_bearer_token_from_file() {
        let path = std::env::temp_dir().join(format!(
            "mcp-cli-token-{}-{}.txt",
            std::process::id(),
            "reads_bearer_token_from_file"
        ));
        std::fs::write(&path, "secret\n").unwrap();

        let cli = Cli::parse_from([
            "mcp",
            "https://example.com/mcp",
            "--bearer-token-file",
            path.to_str().unwrap(),
        ]);
        let config = cli
            .transport_config_with_resolvers(|_| Err(std::env::VarError::NotPresent), no_prompt)
            .unwrap();

        let _ = std::fs::remove_file(path);

        assert_eq!(
            config,
            TransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                headers: Default::default(),
                bearer_token: Some("secret".to_string())
            }
        );
    }

    #[test]
    fn reads_bearer_token_from_prompt() {
        let cli = Cli::parse_from(["mcp", "https://example.com/mcp", "--bearer-token-prompt"]);
        let config = cli
            .transport_config_with_resolvers(
                |_| Err(std::env::VarError::NotPresent),
                || Ok("secret\n".to_string()),
            )
            .unwrap();

        assert_eq!(
            config,
            TransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                headers: Default::default(),
                bearer_token: Some("secret".to_string())
            }
        );
    }

    #[test]
    fn rejects_multiple_bearer_token_sources() {
        let cli = Cli::parse_from([
            "mcp",
            "https://example.com/mcp",
            "--bearer-token-env",
            "MCP_TEST_TOKEN",
            "--bearer-token-prompt",
        ]);
        assert!(cli.transport_config().is_err());
    }

    #[test]
    fn rejects_missing_env_bearer_token() {
        let cli = Cli::parse_from([
            "mcp",
            "https://example.com/mcp",
            "--bearer-token-env",
            "MCP_TEST_TOKEN",
        ]);
        assert!(
            cli.transport_config_with_resolvers(|_| Err(std::env::VarError::NotPresent), no_prompt)
                .is_err()
        );
    }

    #[test]
    fn rejects_empty_env_bearer_token() {
        let cli = Cli::parse_from([
            "mcp",
            "https://example.com/mcp",
            "--bearer-token-env",
            "MCP_TEST_TOKEN",
        ]);
        assert!(
            cli.transport_config_with_resolvers(|_| Ok(" \n".to_string()), no_prompt)
                .is_err()
        );
    }

    #[test]
    fn rejects_empty_file_bearer_token() {
        let path = std::env::temp_dir().join(format!(
            "mcp-cli-token-{}-{}.txt",
            std::process::id(),
            "rejects_empty_file_bearer_token"
        ));
        std::fs::write(&path, "\n").unwrap();

        let cli = Cli::parse_from([
            "mcp",
            "https://example.com/mcp",
            "--bearer-token-file",
            path.to_str().unwrap(),
        ]);
        let result =
            cli.transport_config_with_resolvers(|_| Err(std::env::VarError::NotPresent), no_prompt);

        let _ = std::fs::remove_file(path);

        assert!(result.is_err());
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
