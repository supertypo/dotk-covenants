//! Claims of the dotk API. The verifier checks each one against the compiler or the node.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use silverscript_abi::SilAbiArtifact;

#[derive(Debug, Clone, Deserialize)]
pub struct Params {
    pub devfund_spk: String,
    pub fee_1ch: u64,
    pub fee_2ch: u64,
    pub fee_3ch: u64,
    pub fee_4ch: u64,
    pub fee_5plus: u64,
    pub bond: u64,
    pub deposit: u64,
    pub gap_value: u64,
    pub t_evict: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub network: String,
    pub registry_covenant_id: String,
    pub gap_abi: SilAbiArtifact,
    pub deed_abi: SilAbiArtifact,
    pub gap_template_hash: String,
    pub deed_template_hash: String,
    pub params: Params,
    pub genesis_binding: GenesisBinding,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenesisBinding {
    pub authorizing_outpoint: Outpoint,
    pub output_index: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outpoint {
    pub transaction_id: String,
    pub index: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub registry_covenant_id: String,
    pub deeds: Vec<Deed>,
}

pub const KIND_ACTIVE: u8 = 0;
pub const KIND_PENDING: u8 = 1;
pub const KIND_OWNER_UNKNOWN: u8 = 2;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deed {
    pub key: String,
    pub kind: u8,
    pub name: Option<String>,
    pub owner_type: Option<u8>,
    pub owner: Option<String>,
    pub claim: Option<String>,
}

async fn get<T: serde::de::DeserializeOwned>(http: &reqwest::Client, url: &str) -> Result<T> {
    let res = http.get(url).send().await.with_context(|| format!("GET {url}"))?;
    let status = res.status();
    if !status.is_success() {
        return Err(anyhow!("GET {url} answered {status}"));
    }
    res.json().await.with_context(|| format!("reading the answer of GET {url}"))
}

pub async fn manifest(http: &reqwest::Client, api: &str) -> Result<Manifest> {
    get(http, &format!("{api}/genesis")).await
}

/// The live export, because the verifier does its own proof.
pub async fn snapshot(http: &reqwest::Client, api: &str) -> Result<Snapshot> {
    get(http, &format!("{api}/snapshot?proven=false&events=false")).await
}
