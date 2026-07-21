use liberado_python_interpreter_mcp::{config::Config, constants, server::InterpreterServer};
use turbomcp::prelude::*;

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
        constants::SERVER_NAME,
        config.bind_addr,
    );

    let bind_addr = config.bind_addr.clone();

    InterpreterServer::new(config)
        .builder()
        .allow_any_origin(true)
        .transport(Transport::http(&bind_addr))
        .serve()
        .await?;

    Ok(())
}
