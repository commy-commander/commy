#![allow(deprecated)]

use anyhow::{Context, Result};
use iroh::net::relay::server::{Limits, RelayConfig, Server, ServerConfig, StunConfig};
pub use iroh::net::relay::RelayMap;
use iroh::net::relay::{RelayNode, RelayUrl};
use std::net::Ipv4Addr;

pub struct LocalRelay {
    server: Server,
    pub url: RelayUrl,
    stun_port: u16,
}

impl LocalRelay {
    pub async fn start() -> Result<Self> {
        let config = ServerConfig::<(), ()> {
            relay: Some(RelayConfig {
                http_bind_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                tls: None,
                limits: Limits::default(),
            }),
            stun: Some(StunConfig { bind_addr: (Ipv4Addr::LOCALHOST, 0).into() }),
            ..Default::default()
        };
        let server = Server::spawn(config).await.context("failed to start the local relay")?;
        let http = server.http_addr().context("the local relay has no address")?;
        let stun_port = server.stun_addr().context("the local relay has no STUN address")?.port();
        let url: RelayUrl = format!("http://{http}").parse()?;
        Ok(Self { server, url, stun_port })
    }

    pub fn map(&self) -> RelayMap {
        RelayMap::from_nodes([RelayNode { url: self.url.clone(), stun_only: false, stun_port: self.stun_port }])
            .expect("one relay node is a valid map")
    }

    pub async fn stop(self) {
        self.server.shutdown().await.ok();
    }
}
