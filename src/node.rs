use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Result, anyhow, ensure};
use kaspa_addresses::Address;
use kaspa_rpc_core::RpcHash;
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_wrpc_client::prelude::{ConnectOptions, ConnectStrategy, NetworkId, NetworkType};
use kaspa_wrpc_client::{KaspaRpcClient, Resolver, WrpcEncoding};

/// The most addresses in one `getUtxosByAddresses` request.
const CHUNK: usize = 800;

const RESOLVER_PICKS: usize = 3;

const REQUEST_ATTEMPTS: usize = 3;

fn connect_options() -> ConnectOptions {
    ConnectOptions {
        block_async_connect: true,
        connect_timeout: Some(Duration::from_secs(10)),
        strategy: ConnectStrategy::Fallback,
        ..Default::default()
    }
}

pub struct Node {
    client: KaspaRpcClient,
    pub url: String,
    pub version: String,
    /// Given with -s, so a replacement reconnects to it rather than asking the resolver.
    explicit: bool,
}

impl Node {
    pub async fn connect(url: Option<&str>) -> Result<Self> {
        if let Some(url) = url {
            return Self::connect_to(url.to_string(), true).await;
        }
        let network = NetworkId::new(NetworkType::Mainnet);
        let mut last = anyhow!("the resolver gave no node");
        for _ in 0..RESOLVER_PICKS {
            match Resolver::default().get_url(WrpcEncoding::Borsh, network).await {
                Ok(url) => match Self::connect_to(url, false).await {
                    Ok(node) => return Ok(node),
                    Err(e) => last = e,
                },
                Err(e) => last = anyhow!("resolver: {e}"),
            }
        }
        Err(last)
    }

    async fn connect_to(url: String, explicit: bool) -> Result<Self> {
        let network = NetworkId::new(NetworkType::Mainnet);
        let client = KaspaRpcClient::new(WrpcEncoding::Borsh, Some(&url), None, Some(network), None)?;
        client.connect(Some(connect_options())).await.map_err(|e| anyhow!("connecting to {url}: {e}"))?;
        let info = client.get_server_info().await.map_err(|e| anyhow!("getServerInfo on {url}: {e}"))?;
        ensure!(info.network_id == network, "the node at {url} is on {}, not mainnet", info.network_id);
        ensure!(info.is_synced, "the node at {url} is not synced");
        ensure!(info.has_utxo_index, "the node at {url} runs without --utxoindex, so it cannot look up addresses");
        Ok(Self { client, url, version: info.server_version, explicit })
    }

    /// The same node if it still answers. Otherwise a new connection, because a node can drop a
    /// connection that waited at a prompt.
    pub async fn alive(self) -> Result<Self> {
        if tokio::time::timeout(Duration::from_secs(10), self.client.get_server_info()).await.is_ok_and(|info| info.is_ok()) {
            return Ok(self);
        }
        let explicit = self.explicit.then(|| self.url.clone());
        self.disconnect().await;
        Self::connect(explicit.as_deref()).await
    }

    pub async fn holding(&self, addresses: &[Address], covenant_id: RpcHash) -> Result<HashSet<Address>> {
        let mut found = HashSet::new();
        for chunk in addresses.chunks(CHUNK) {
            let entries = self.utxos(chunk).await?;
            let held =
                dotk_covenants::verify::holding(entries.into_iter().map(|e| (e.address, e.utxo_entry.covenant_id)), covenant_id)
                    .map_err(|e| e.context(format!("the node at {}", self.url)))?;
            found.extend(held);
        }
        Ok(found)
    }

    /// Public nodes drop connections under load.
    async fn utxos(&self, chunk: &[Address]) -> Result<Vec<kaspa_rpc_core::RpcUtxosByAddressesEntry>> {
        let mut attempt = 1;
        loop {
            match self.client.get_utxos_by_addresses(chunk.to_vec()).await {
                Ok(entries) => return Ok(entries),
                Err(e) if attempt == REQUEST_ATTEMPTS => return Err(anyhow!("getUtxosByAddresses on {}: {e}", self.url)),
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    if !self.client.is_connected() {
                        let _ = self.client.connect(Some(connect_options())).await;
                    }
                    attempt += 1;
                }
            }
        }
    }

    /// Bounded, because a close on a dead socket can hang.
    pub async fn disconnect(self) {
        let _ = tokio::time::timeout(Duration::from_secs(3), self.client.disconnect()).await;
    }
}
