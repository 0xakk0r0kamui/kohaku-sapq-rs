use alloy::{providers::Provider, rpc::types::Filter};
use kohaku_rpc_provider::{StreamedEvent, StreamedEventConfig};
use tracing::{info, warn};

use crate::{
    pool::Pool,
    syncer::{Snapshot, SyncEvent, Syncer, SyncerError},
};

/// A syncer that reads from an Ethereum JSON-RPC provider
#[derive(Clone)]
pub struct RpcSyncer<P: Provider> {
    provider: P,
    config: StreamedEventConfig,
}

#[derive(Debug, thiserror::Error)]
enum RpcSyncerError {
    #[error("Error decoding log: {0}")]
    LogDecodeError(#[from] alloy::sol_types::Error),
    #[error("RPC error: {0}")]
    RpcError(#[from] alloy::transports::RpcError<alloy::transports::TransportErrorKind>),
    #[error("Field conversion error: {0}")]
    Field(#[from] crate::field::NotInRangeError),
}

impl<P: Provider> RpcSyncer<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            config: StreamedEventConfig::default(),
        }
    }

    /// Sets the chunking, concurrency and backoff options for `eth_getLogs` calls.
    #[must_use]
    pub fn with_config(mut self, config: StreamedEventConfig) -> Self {
        self.config = config;
        self
    }
}

#[cfg_attr(native, async_trait::async_trait)]
#[cfg_attr(wasm, async_trait::async_trait(?Send))]
impl<P: Provider + Clone> Syncer for RpcSyncer<P> {
    async fn sync_range(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<Snapshot, SyncerError> {
        Ok(self
            .sync(pool, from_block, to_block)
            .await
            .map_err(SyncerError::other)?)
    }
}

impl<P: Provider + Clone> RpcSyncer<P> {
    async fn sync(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<Snapshot, RpcSyncerError> {
        let range = self.block_range(pool, from_block, to_block).await?;
        info!("Syncing from {} to {}", range.start, range.end);

        if range.is_empty() {
            return Ok(Snapshot::new(pool.clone(), range, Vec::new()));
        }

        //? `eth_getLogs` treats both block bounds as inclusive
        let filter = Filter::new()
            .address(pool.address)
            .from_block(range.start)
            .to_block(range.end - 1);
        let logs = StreamedEvent::new(self.provider.clone(), filter)
            .config(self.config)
            .query()
            .await?;

        let mut events = Vec::new();
        for log in logs {
            match SyncEvent::try_from_log(&log) {
                Some(decoded) => events.push(decoded),
                None => {
                    warn!("Unknown event with topic {:?}", log.topic0());
                }
            }
        }

        info!("Synced {} events", events.len());
        Ok(Snapshot::new(pool.clone(), range, events))
    }

    /// Clamps the requested range to the blocks this syncer can serve: the pool's deployment
    /// block through the chain's latest block.
    async fn block_range(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<std::ops::Range<u64>, RpcSyncerError> {
        let from = from_block.max(pool.deployed_block);
        //? `latest_block` is an inclusive block number, `to_block` an exclusive bound.
        let latest = self.provider.get_block_number().await?.saturating_add(1);
        let to = to_block.min(latest).max(from);

        Ok(from..to)
    }
}
