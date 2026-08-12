mod auth;
mod config;
mod db;
mod model;
mod sql;
mod ui;
mod web;

use anyhow::Result;
use auth::AuthState;
use config::Settings;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::EnvFilter;
use web::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        return process_healthcheck();
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "openapi_fdw_control=info".into()),
        )
        .compact()
        .init();

    let settings = Settings::from_environment()?;
    let pool = db::create_pool(&settings.database_url, settings.pool_max_size)?;
    db::bootstrap(&pool).await?;
    db::health(&pool).await?;

    let state = AppState {
        pool,
        auth: Arc::new(AuthState::new(settings.admin_token, settings.cookie_secure)),
    };
    let app = web::router(state);
    let listener = tokio::net::TcpListener::bind(settings.listen).await?;
    info!(listen = %settings.listen, "OpenAPI FDW control plane is ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn process_healthcheck() -> Result<()> {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let configured: SocketAddr = std::env::var("OPENAPI_FDW_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;
    let address = SocketAddr::from(([127, 0, 0, 1], configured.port()));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut response = [0_u8; 64];
    let count = stream.read(&mut response)?;
    let status = std::str::from_utf8(&response[..count])?;
    if !status.starts_with("HTTP/1.1 200") {
        anyhow::bail!("control-plane health endpoint is not ready");
    }
    Ok(())
}

async fn shutdown_signal() {
    let control_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = control_c => {},
        () = terminate => {},
    }
}
