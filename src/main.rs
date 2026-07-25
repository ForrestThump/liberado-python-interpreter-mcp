use std::process::ExitCode;

use liberado_python_interpreter_mcp::{
    config::Config, constants, sandbox, server::InterpreterServer,
};
use turbomcp::http::{serve_http, HttpConfig};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| constants::DEFAULT_LOG_LEVEL.into()),
        )
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Refuse to serve on bad configuration rather than failing every tool call later.
            tracing::error!(error = %e, "{} failed to start", constants::SERVER_NAME);
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let addr = config.socket_addr()?;

    // State the effective isolation mode at boot. It used to be discoverable only from a
    // per-session warning, so an operator could believe sessions were jailed when they were not.
    if config.sandbox_enabled {
        match sandbox::probe_sandbox(&config) {
            Ok(path) => tracing::info!(nsjail = %path, "Per-session nsjail sandbox is active"),
            Err(e) if config.sandbox_required => return Err(Box::new(e)),
            Err(e) => tracing::warn!(
                error = %e,
                "nsjail unavailable — sessions will run as direct child processes; the container \
                 is the isolation boundary. Set LIBERADO_SANDBOX_REQUIRED=1 to refuse instead."
            ),
        }
    } else {
        tracing::info!(
            "nsjail disabled by configuration — the container is the isolation boundary"
        );
    }

    tracing::info!(
        "{} v{} listening on {} ({})",
        constants::SERVER_NAME,
        constants::SERVER_VERSION,
        config.bind_addr,
        config.summary(),
    );

    let server = InterpreterServer::new(config);
    let _reaper = server.spawn_reaper();
    serve_http(addr, server.into_server().build(), HttpConfig::new()).await?;
    Ok(())
}
