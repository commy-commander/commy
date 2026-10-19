use anyhow::{Context, Result};
use iroh::{RelayConfig, RelayUrl};
pub use iroh::RelayMap;
use iroh_relay::server::{RelayConfig as ServerRelayConfig, Server, ServerConfig};
use std::net::Ipv4Addr;

pub struct LocalRelay {
    server: Server,
    pub url: RelayUrl,
}

impl LocalRelay {
    pub async fn start() -> Result<Self> {
        let mut config = ServerConfig::default();
        config.relay = Some(ServerRelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
        let server = Server::spawn(config).await.context("failed to start the local relay")?;
        let http = server.http_addr().context("the local relay has no address")?;
        let url: RelayUrl = format!("http://{http}").parse()?;
        Ok(Self { server, url })
    }

    pub fn map(&self) -> RelayMap {
        RelayConfig::new(self.url.clone(), None).into()
    }

    pub async fn stop(self) {
        self.server.shutdown().await.ok();
    }
}
