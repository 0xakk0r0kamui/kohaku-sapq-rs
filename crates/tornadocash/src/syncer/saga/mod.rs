use thiserror::Error;
use tracing::info;

use self::{
    decode::{parse_chunk_events, verify_and_decompress_chunk},
    manifest::Manifest,
};
use crate::{
    pool::Pool,
    syncer::{Snapshot, Syncer, SyncerError},
};

mod decode;
mod manifest;

/// A syncer that reads pre-scraped event chunks published under the
/// [saga-sync](https://github.com/fatlabsxyz/saga-sync) protocol.
///
/// Fetches the protocol's manifest, verifies each overlapping chunk's sha256 digest against it,
/// and decodes the chunk's events into [`SyncEvent`](crate::syncer::SyncEvent)s.
///
/// Does not currently implement chunk caching or chunk signature verification.
/// - Chunk caching could be added to reduce redundant network downloads, but is not required for
///   this MVP.
/// - I don't fully buy the benefits of signature verification. Since invalid chunks would result in
///   invalid merkle roots, we can detect and skip invalid chunks regardless.
pub struct SagaSyncer {
    client: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Error)]
enum SagaSyncError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Hex decode error: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("Gzip decode error: {0}")]
    Gzip(#[from] std::io::Error),
    #[error("Chunk is not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Event decode error: {0}")]
    EventDecode(#[from] alloy::sol_types::Error),
    #[error("Chunk digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("Chunk events out of order at line {index}")]
    OutOfOrder { index: usize },
    #[error("Event at block {block} outside chunk range [{from}, {to})")]
    OutOfRange { block: u64, from: u64, to: u64 },
    #[error("Field conversion error: {0}")]
    Field(#[from] crate::field::NotInRangeError),
    #[error("Missing manifest entry for pool {0}")]
    MissingManifestEntry(Pool),
}

impl SagaSyncer {
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self::with_client(reqwest::Client::new(), base_url)
    }

    /// `client` is the HTTP client for the manifest and chunk fetches.
    ///
    /// A later caller can pass a client that uses a proxy without this syncer knowing why.
    #[must_use]
    pub fn with_client(client: reqwest::Client, base_url: &str) -> Self {
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }
}

#[cfg_attr(native, async_trait::async_trait)]
#[cfg_attr(wasm, async_trait::async_trait(?Send))]
impl Syncer for SagaSyncer {
    async fn sync_range(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<Snapshot, SyncerError> {
        self.sync(pool, from_block, to_block)
            .await
            .map_err(SyncerError::other)
    }
}

impl SagaSyncer {
    async fn sync(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<Snapshot, SagaSyncError> {
        let manifest = self.fetch_manifest().await?;
        let key = stream_key(pool);

        let from = from_block.max(pool.deployed_block);
        let Some(entry) = manifest.available_protocols.get(&key) else {
            return Err(SagaSyncError::MissingManifestEntry(pool.clone()));
        };

        //? Chunk bounds are already half-open, so the last chunk's `to_block` is the exclusive
        //? end of everything this stream publishes.
        let published = entry.last_block().unwrap_or(pool.deployed_block);
        let range = from..to_block.min(published).max(from);

        info!("Syncing from {} to {}", range.start, range.end);

        let overlapping = entry
            .chunks
            .iter()
            .chain(entry.hot_head.iter())
            .filter(|c| c.from_block < range.end && c.to_block > range.start);

        let mut events = Vec::new();
        for chunk in overlapping {
            let gz_bytes = self.fetch_chunk_bytes(&chunk.file).await?;

            let decompressed = verify_and_decompress_chunk(&gz_bytes, chunk)?;

            let decoded = parse_chunk_events(&decompressed, chunk, range.clone())?;

            events.extend(decoded);
        }

        Ok(Snapshot::new(pool.clone(), range, events))
    }

    async fn fetch_manifest(&self) -> Result<Manifest, SagaSyncError> {
        let bytes = self
            .client
            .get(self.manifest_url())
            .send()
            .await?
            .bytes()
            .await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn fetch_chunk_bytes(&self, file: &str) -> Result<Vec<u8>, SagaSyncError> {
        let bytes = self
            .client
            .get(self.chunk_url(file))
            .send()
            .await?
            .bytes()
            .await?;
        Ok(bytes.to_vec())
    }

    fn manifest_url(&self) -> String {
        format!("{}/index.json", self.base_url)
    }

    fn chunk_url(&self, file: &str) -> String {
        format!("{}/{}", self.base_url, file)
    }
}

#[must_use]
fn stream_key(pool: &Pool) -> String {
    format!(
        "tornado-cash-{}-{}-{}",
        pool.chain_id,
        pool.symbol(),
        pool.amount()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_stream_key() {
        assert_eq!(
            stream_key(&Pool::ETHEREUM_ETHER_01),
            "tornado-cash-1-eth-0.1"
        );
    }
}
