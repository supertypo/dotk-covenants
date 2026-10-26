//! Claims of the dotk market API. The verifier checks each one against the compiler or the node.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reserve {
    pub txid: String,
    pub index: u32,
    pub daa_score: Option<u64>,
}

/// One served listing. The market serves listings of more than one escrow version, and only
/// version 2 carries `fee`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub name: String,
    pub key: String,
    pub owner_hash: String,
    pub version: u8,
    pub price: u64,
    #[serde(default)]
    pub fee: Option<u64>,
    pub window: u64,
    pub seller_type: u8,
    pub seller: String,
    pub seller_address: String,
    pub venue_type: u8,
    pub venue: String,
    pub venue_address: String,
    pub salt_txid: String,
    pub salt_index: u32,
    pub registry_covenant_id: String,
    pub deed_address: String,
    pub escrow_address: String,
    pub listed_txid: String,
    pub reserve: Reserve,
    pub expires_daa_score: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    listings: Vec<Listing>,
    next: Option<String>,
}

async fn get<T: serde::de::DeserializeOwned>(http: &reqwest::Client, url: &str) -> Result<Option<T>> {
    let res = http.get(url).send().await.with_context(|| format!("GET {url}"))?;
    let status = res.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(anyhow!("GET {url} answered {status}"));
    }
    res.json().await.map(Some).with_context(|| format!("reading the answer of GET {url}"))
}

/// Every listing that the market serves, page by page.
pub async fn listings(http: &reqwest::Client, api: &str) -> Result<Vec<Listing>> {
    let base = reqwest::Url::parse(&format!("{api}/listings"))?;
    let mut all = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut url = base.clone();
    loop {
        let page: Page = get(http, url.as_str()).await?.ok_or_else(|| anyhow!("GET {url} answered 404"))?;
        all.extend(page.listings);
        let Some(after) = page.next else { return Ok(all) };
        if !seen.insert(after.clone()) {
            return Err(anyhow!("GET {url} names the page after {after} a second time"));
        }
        url = base.clone();
        url.query_pairs_mut().append_pair("after", &after);
    }
}

/// The listing of one name, or nothing if the market serves none.
pub async fn listing(http: &reqwest::Client, api: &str, name: &str) -> Result<Option<Listing>> {
    let mut url = reqwest::Url::parse(&format!("{api}/listings"))?;
    url.path_segments_mut().map_err(|()| anyhow!("{api} cannot take a path"))?.push(name);
    get(http, url.as_str()).await
}
