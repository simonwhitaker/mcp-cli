use anyhow::Result;
use clap::Parser;
use mcp_cli::{cli::Cli, format::Formatter, session::McpSession, shell::Shell};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.debug);

    let transport = cli.transport_config()?;
    let formatter = Formatter::new(!cli.no_color, cli.json);
    let mut session = McpSession::connect(&transport, cli.debug).await?;
    let server_name = {
        let server_info = session.server_info()?;
        println!(
            "{}",
            formatter.server_intro(&server_info, session.tools().len())
        );
        server_info.server_info.name.to_string()
    };

    let mut shell = Shell::new(&server_name, &session, cli.history, formatter)?;
    let result = shell.run(&mut session).await;
    session.close().await?;
    result
}

fn init_tracing(debug: bool) {
    let filter = if debug {
        EnvFilter::from_default_env().add_directive("mcp_cli=debug".parse().unwrap())
    } else {
        EnvFilter::from_default_env().add_directive("warn".parse().unwrap())
    };

    let _ = fmt()
        .with_env_filter(filter)
        .with_target(debug)
        .with_writer(std::io::stderr)
        .try_init();
}
