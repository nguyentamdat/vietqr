use sqlx::postgres::PgPoolOptions;
use vietqr_server::{AppState, MIGRATOR, create_key, router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let create = match (args.next().as_deref(), args.next()) {
        (None, None) => false,
        (Some("create-key"), None) => true,
        _ => return Err("Usage: vietqr-server [create-key]".into()),
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&std::env::var("DATABASE_URL")?)
        .await?;
    MIGRATOR.run(&pool).await?;
    if create {
        println!("{}", create_key(&pool).await?);
        return Ok(());
    }
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("vietqr-server listening on {}", listener.local_addr()?);
    let mut state = AppState::new(pool);
    state.trusted_proxy_cidrs = match std::env::var("TRUSTED_PROXY_CIDRS") {
        Ok(value) if !value.trim().is_empty() => value
            .split(',')
            .map(|cidr| cidr.trim().parse())
            .collect::<Result<_, _>>()?,
        Ok(_) | Err(std::env::VarError::NotPresent) => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .expect("install ctrl-c handler");
}
