use std::net::SocketAddr;

use recovery_api::{AppConfig, build_router};
use recovery_persistence::Store;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://recovery-control-room.db?mode=rwc".to_owned());
    let allowed_origin =
        std::env::var("ALLOWED_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".to_owned());
    let secure_cookie = std::env::var("SECURE_COOKIE").is_ok_and(|value| value == "true");
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);

    let store = Store::connect(&database_url).await?;
    let app = build_router(
        store,
        AppConfig {
            allowed_origin,
            secure_cookie,
        },
    );
    let address = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "recovery API listening");
    axum::serve(listener, app).await?;
    Ok(())
}
