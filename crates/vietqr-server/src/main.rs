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
    axum::serve(listener, router(AppState::new(pool)))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
