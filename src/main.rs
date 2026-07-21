mod server;

use server::InterpreterServer;
use turbomcp::prelude::*;

use liberado_python_interpreter_mcp::config::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env();

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| config.log_level.as_str().into()),
        )
        .init();

    tracing::info!(
        "{} listening on {}",
        liberado_python_interpreter_mcp::constants::SERVER_NAME,
        config.bind_addr,
    );

    InterpreterServer::new(config)
        .builder()
        .allow_any_origin(true)
        .transport(Transport::http(&config.bind_addr))
        .serve()
        .await?;

    Ok(())
}
