use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "mcp",
    version,
    about = "Inspect and debug MCP servers from a terminal REPL",
    trailing_var_arg = true
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
        value_name = "SERVER_COMMAND",
        required = true,
        num_args = 1..,
        help = "Command and arguments that expose an MCP server over stdio"
    )]
    pub server_command: Vec<String>,
}
