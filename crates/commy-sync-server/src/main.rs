use commy_sync_server::apns::{Apns, ApnsConfig};
use commy_sync_server::config::{Config, KeySeed};
use commy_sync_server::fcm::Fcm;
use commy_sync_server::store::Store;
use commy_sync_server::push_via::ViaGateway;
use commy_sync_server::wake::Pusher;
use commy_sync_server::{learn_dropped, operator, serve, spawn_housekeeping, Service};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let words: Vec<String> = std::env::args().skip(1).collect();
    if words.first().map(String::as_str) == Some("check") {
        return check(&words[1..]).await;
    }
    if words == ["report-key"] {
        print!("{}", operator::new_report_key());
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let mut config = Config::from_env(|key| std::env::var(key).ok()).map_err(|e| anyhow::anyhow!("invalid sync service configuration: {e}"))?;
    if !words.is_empty() {
        return run_operator_command(&config, &words).await;
    }
    raise_open_file_limit();
    let store = match &config.database_url {
        Some(url) => Store::postgres_pooled(url, config.db_pool).await?,
        None => {
            tracing::warn!("DATABASE_URL is not set, so entries live in memory and are lost on restart");
            Store::memory()
        }
    };
    let fcm = match &config.fcm_service_account {
        Some(account) => Some(Fcm::from_config(account).map_err(|e| anyhow::anyhow!("invalid FCM_SERVICE_ACCOUNT: {e}"))?),
        None => None,
    };
    let apns = match &config.apns {
        Some(s) => Some(
            Apns::from_config(ApnsConfig { key: &s.key, key_id: &s.key_id, team_id: &s.team_id, topic: &s.topic })
                .map_err(|e| anyhow::anyhow!("invalid Apple push settings: {e}"))?,
        ),
        None => None,
    };
    let server_key = commy_sync_server::server_key::load(&config, &store).await?;
    config.server_key = Some(KeySeed(server_key.to_bytes()));
    tracing::info!("sync server key {}", commy_sync_server::server_key::published(&server_key).id);
    let pusher = match (Pusher::live(fcm, apns), &config.push_via) {
        (Pusher::Off, Some(via)) => {
            tracing::info!("waking phones through the push gateway at {} as {}", via.gateway, via.origin);
            Pusher::Gateway(Box::new(ViaGateway::new(&via.gateway, &via.origin, commy_core::Identity::from_bytes(&server_key.to_bytes()))))
        }
        (pusher, _) => pusher,
    };
    let service = Service::with_pusher(store, &config, pusher);
    learn_dropped(&service).await;
    spawn_housekeeping(service.clone());
    commy_sync_server::passes::spawn_refresh(service.clone());
    if let Some(settings) = config.admin.clone() {
        let admin = tokio::net::TcpListener::bind(settings.listen).await?;
        tracing::info!("admin page listening on port {}", settings.listen.port());
        let service = service.clone();
        tokio::spawn(async move {
            if let Err(e) = commy_sync_server::admin::serve(admin, service, settings).await {
                tracing::error!("admin page stopped: {e}");
            }
        });
    }
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!("sync service listening: {}", config.summary());
    tokio::select! {
        result = serve(listener, service) => result?,
        _ = stop_signal() => tracing::info!("sync service stopping"),
    }
    Ok(())
}

async fn check(args: &[String]) -> anyhow::Result<()> {
    let usage = || anyhow::anyhow!("usage: commy-sync check wss://<domain>/sync [--ca <root certificate file>]");
    let (url, roots) = match args {
        [url] => (url, None),
        [url, flag, path] if flag == "--ca" => (url, Some(std::path::Path::new(path))),
        _ => return Err(usage()),
    };
    let checked = commy_sync_server::check::run(url, roots).await.map_err(|e| anyhow::anyhow!("{url}: {e}"))?;
    println!("ok: {url} stored and returned entry {} (sync protocol {})", checked.seq, checked.protocol);
    Ok(())
}

async fn run_operator_command(config: &Config, words: &[String]) -> anyhow::Result<()> {
    let url = config.database_url.as_deref().ok_or_else(|| anyhow::anyhow!("operator commands need DATABASE_URL"))?;
    if let [open, id] = words {
        if open == "open" {
            let mut key = String::new();
            std::io::Read::read_to_string(&mut std::io::Read::take(std::io::stdin(), 4096), &mut key)?;
            let store = Store::postgres(url).await?;
            print!("{}", operator::open(&store, id, &key).await?);
            return Ok(());
        }
    }
    let command = operator::Command::parse(words).map_err(|usage| anyhow::anyhow!("{usage}"))?;
    let store = Store::postgres(url).await?;
    print!("{}", operator::run(&store, command).await?);
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
