use std::net::SocketAddr;

use liberado_python_interpreter_mcp::{config::Config, constants, server::InterpreterServer};
use turbomcp::http::{serve_http, HttpConfig};

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

    let addr: SocketAddr = config
        .bind_addr
        .parse()
        .expect("BIND_ADDR must be a valid SocketAddr");
    let service = InterpreterServer::new(config).into_server().build();
    serve_http(addr, service, HttpConfig::new()).await?;

    Ok(())
}
