mod server;

use server::InterpreterServer;
use turbomcp::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8000".to_string());
    tracing::info!("liberado-python-interpreter-mcp listening on {addr}");

    InterpreterServer::new()
        .builder()
        .allow_any_origin(true)
        .transport(Transport::http(&addr))
        .serve()
        .await?;

    Ok(())
}
