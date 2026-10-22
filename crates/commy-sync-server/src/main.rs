use commy_sync_server::config::Config;
use commy_sync_server::fcm::Fcm;
use commy_sync_server::store::Store;
use commy_sync_server::wake::Pusher;
use commy_sync_server::{serve, spawn_housekeeping, Service};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    raise_open_file_limit();
    let config = Config::from_env(|key| std::env::var(key).ok()).map_err(|e| anyhow::anyhow!("invalid sync service configuration: {e}"))?;
    let store = match &config.database_url {
        Some(url) => Store::postgres(url).await?,
        None => {
            tracing::warn!("DATABASE_URL is not set, so entries live in memory and are lost on restart");
            Store::memory()
        }
    };
    let pusher = match &config.fcm_service_account {
        Some(account) => Pusher::Fcm(Box::new(Fcm::from_config(account).map_err(|e| anyhow::anyhow!("invalid FCM_SERVICE_ACCOUNT: {e}"))?)),
        None => Pusher::Off,
    };
    let service = Service::with_pusher(store, &config, pusher);
    spawn_housekeeping(service.clone());
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!("sync service listening: {}", config.summary());
    tokio::select! {
        result = serve(listener, service) => result?,
        _ = stop_signal() => tracing::info!("sync service stopping"),
    }
    Ok(())
}

#[cfg(unix)]
fn raise_open_file_limit() {
    match rlimit::increase_nofile_limit(u64::MAX) {
        Ok(limit) => tracing::info!("sync service may hold {limit} open files"),
        Err(e) => tracing::warn!("could not raise the open file limit: {e}"),
    }
}

#[cfg(not(unix))]
fn raise_open_file_limit() {}

#[cfg(unix)]
async fn stop_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let Ok(mut term) = signal(SignalKind::terminate()) else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

#[cfg(not(unix))]
async fn stop_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
