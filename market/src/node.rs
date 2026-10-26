use std::time::Duration;

use anyhow::{Result, anyhow, ensure};
use dotk_listing::verify::Utxo;
use dotk_registry::verify::Network;
use kaspa_addresses::Address;
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_wrpc_client::prelude::{ConnectOptions, ConnectStrategy, NetworkId, NetworkType};
use kaspa_wrpc_client::{KaspaRpcClient, Resolver, WrpcEncoding};

/// The most addresses in one `getUtxosByAddresses` request.
const CHUNK: usize = 800;

const RESOLVER_PICKS: usize = 3;

const REQUEST_ATTEMPTS: usize = 3;

fn network_id(on: &Network) -> Result<NetworkId> {
    match on.name {
        "mainnet" => Ok(NetworkId::new(NetworkType::Mainnet)),
        "testnet-10" => Ok(NetworkId::with_suffix(NetworkType::Testnet, 10)),
        other => Err(anyhow!("no node network for {other}")),
    }
}

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
    network: &'static Network,
}

impl Node {
    pub async fn connect(url: Option<&str>, on: &'static Network) -> Result<Self> {
        if let Some(url) = url {
            return Self::connect_to(url.to_string(), true, on).await;
        }
        let network = network_id(on)?;
        let mut last = anyhow!("the resolver gave no node");
        for _ in 0..RESOLVER_PICKS {
            match Resolver::default().get_url(WrpcEncoding::Borsh, network).await {
                Ok(url) => match Self::connect_to(url, false, on).await {
                    Ok(node) => return Ok(node),
                    Err(e) => last = e,
                },
                Err(e) => last = anyhow!("resolver: {e}"),
            }
        }
        Err(last)
    }

    async fn connect_to(url: String, explicit: bool, on: &'static Network) -> Result<Self> {
        let network = network_id(on)?;
        let client = KaspaRpcClient::new(WrpcEncoding::Borsh, Some(&url), None, Some(network), None)?;
        client.connect(Some(connect_options())).await.map_err(|e| anyhow!("connecting to {url}: {e}"))?;
        let info = client.get_server_info().await.map_err(|e| anyhow!("getServerInfo on {url}: {e}"))?;
        ensure!(info.network_id == network, "the node at {url} is on {}, not {}", info.network_id, on.name);
        ensure!(info.is_synced, "the node at {url} is not synced");
        ensure!(info.has_utxo_index, "the node at {url} runs without --utxoindex, so it cannot look up addresses");
        Ok(Self { client, url, version: info.server_version, explicit, network: on })
    }

    /// The same node if it still answers. Otherwise a new connection, because a node can drop a
    /// connection that waited at a prompt.
    pub async fn alive(self) -> Result<Self> {
        if tokio::time::timeout(Duration::from_secs(10), self.client.get_server_info()).await.is_ok_and(|info| info.is_ok()) {
            return Ok(self);
        }
        let explicit = self.explicit.then(|| self.url.clone());
        let network = self.network;
        self.disconnect().await;
        Self::connect(explicit.as_deref(), network).await
    }

    /// Every UTXO at `addresses`. An entry without its address is a malformed answer.
    pub async fn utxos_at(&self, addresses: &[Address]) -> Result<Vec<Utxo>> {
        let mut out = Vec::new();
        for chunk in addresses.chunks(CHUNK) {
            for e in self.utxos(chunk).await? {
                let address =
                    e.address.ok_or_else(|| anyhow!("the node at {} answered a UTXO entry without its address", self.url))?;
                out.push(Utxo {
                    address,
                    txid: e.outpoint.transaction_id.as_bytes(),
                    index: e.outpoint.index,
                    amount: e.utxo_entry.amount,
                    daa_score: e.utxo_entry.block_daa_score,
                    covenant_id: e.utxo_entry.covenant_id.map(|id| id.as_bytes()),
                });
            }
        }
        Ok(out)
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
