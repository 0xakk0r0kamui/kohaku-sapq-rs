use std::{num::NonZeroU64, ops::RangeInclusive, sync::Arc};

use alloy::{
    primitives::{Address, B256, U256},
    providers::{DynProvider, Provider},
    rpc::types::Filter,
    sol_types::SolEvent,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

use crate::{
    contracts::{AnnouncementEvent, Deployment},
    scheme3::{Announcement, SCHEME_ID, SchemeError},
};

const DEFAULT_MAX_BLOCK_RANGE: NonZeroU64 =
    NonZeroU64::new(10_000).expect("default block range is non-zero");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnouncementRecord {
    stealth_address: [u8; 20],
    caller: [u8; 20],
    ephemeral_public_key: Vec<u8>,
    metadata: Vec<u8>,
    block_number: u64,
    block_hash: [u8; 32],
    transaction_hash: [u8; 32],
    log_index: u64,
}

impl AnnouncementRecord {
    #[must_use]
    pub fn new(
        announcement: &Announcement,
        caller: Address,
        block_number: u64,
        block_hash: B256,
        transaction_hash: B256,
        log_index: u64,
    ) -> Self {
        Self {
            stealth_address: announcement.stealth_address().into_array(),
            caller: caller.into_array(),
            ephemeral_public_key: announcement.ephemeral_public_key().to_vec(),
            metadata: announcement.metadata().to_vec(),
            block_number,
            block_hash: block_hash.0,
            transaction_hash: transaction_hash.0,
            log_index,
        }
    }

    #[must_use]
    pub fn stealth_address(&self) -> Address {
        Address::from(self.stealth_address)
    }

    #[must_use]
    pub fn caller(&self) -> Address {
        Address::from(self.caller)
    }

    #[must_use]
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    #[must_use]
    pub fn block_hash(&self) -> B256 {
        B256::from(self.block_hash)
    }

    #[must_use]
    pub fn transaction_hash(&self) -> B256 {
        B256::from(self.transaction_hash)
    }

    #[must_use]
    pub const fn log_index(&self) -> u64 {
        self.log_index
    }

    /// Returns the validated announcement carried by this log.
    ///
    /// # Errors
    ///
    /// Returns [`SchemeError::Malformed`] if persisted fields fail scheme 3 validation.
    pub fn announcement(&self) -> Result<Announcement, SchemeError> {
        Announcement::from_parts(
            self.stealth_address(),
            self.ephemeral_public_key.clone(),
            self.metadata.clone(),
        )
    }

    pub(crate) fn is_valid(&self) -> bool {
        crate::scheme::announcement_is_valid(
            &self.stealth_address,
            &self.ephemeral_public_key,
            &self.metadata,
        )
    }

    pub(crate) fn id(&self) -> ([u8; 32], u64) {
        (self.block_hash, self.log_index)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchResult {
    pub announcements: Vec<AnnouncementRecord>,
    pub rejected_logs: usize,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SyncerError {
    #[error("announcement RPC request failed")]
    Rpc(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("RPC is connected to chain {actual}; deployment expects chain {expected}")]
    WrongChain { expected: u64, actual: u64 },
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl SyncerError {
    pub fn other(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Other(Box::new(error))
    }
}

fn rpc(error: impl std::error::Error + Send + Sync + 'static) -> SyncerError {
    SyncerError::Rpc(Box::new(error))
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait AnnouncementSyncerBackend: Send + Sync {
    /// Returns the latest block available from this source.
    ///
    /// # Errors
    ///
    /// Returns a source-specific error if the head cannot be read.
    async fn latest_block(&self) -> Result<u64, SyncerError>;

    /// Fetches scheme 3 announcements in an inclusive block range.
    ///
    /// # Errors
    ///
    /// Returns a source-specific error if the range cannot be fetched.
    async fn fetch_announcements(
        &self,
        range: RangeInclusive<u64>,
    ) -> Result<FetchResult, SyncerError>;
}

#[derive(Clone)]
pub struct AnnouncementSyncer(Arc<dyn AnnouncementSyncerBackend>);

impl AnnouncementSyncer {
    #[must_use]
    pub fn new(backend: impl AnnouncementSyncerBackend + 'static) -> Self {
        Self(Arc::new(backend))
    }

    /// Returns the latest block available from the configured source.
    ///
    /// # Errors
    ///
    /// Returns a source-specific error if the head cannot be read.
    pub async fn latest_block(&self) -> Result<u64, SyncerError> {
        self.0.latest_block().await
    }

    /// Fetches scheme 3 announcements in an inclusive block range.
    ///
    /// # Errors
    ///
    /// Returns a source-specific error if the range cannot be fetched.
    pub async fn fetch_announcements(
        &self,
        range: RangeInclusive<u64>,
    ) -> Result<FetchResult, SyncerError> {
        self.0.fetch_announcements(range).await
    }
}

impl<T: AnnouncementSyncerBackend + 'static> From<T> for AnnouncementSyncer {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

#[derive(Clone)]
pub struct RpcAnnouncementSyncer {
    provider: DynProvider,
    chain_id: u64,
    announcer: Address,
    max_block_range: NonZeroU64,
}

impl RpcAnnouncementSyncer {
    #[must_use]
    pub fn new(provider: DynProvider, deployment: Deployment) -> Self {
        Self {
            provider,
            chain_id: deployment.chain_id,
            announcer: deployment.announcer,
            max_block_range: DEFAULT_MAX_BLOCK_RANGE,
        }
    }

    #[must_use]
    pub const fn with_max_block_range(mut self, max_block_range: NonZeroU64) -> Self {
        self.max_block_range = max_block_range;
        self
    }

    async fn ensure_chain(&self) -> Result<(), SyncerError> {
        let actual = self.provider.get_chain_id().await.map_err(rpc)?;
        if actual != self.chain_id {
            return Err(SyncerError::WrongChain {
                expected: self.chain_id,
                actual,
            });
        }
        Ok(())
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl AnnouncementSyncerBackend for RpcAnnouncementSyncer {
    async fn latest_block(&self) -> Result<u64, SyncerError> {
        self.ensure_chain().await?;
        self.provider.get_block_number().await.map_err(rpc)
    }

    async fn fetch_announcements(
        &self,
        range: RangeInclusive<u64>,
    ) -> Result<FetchResult, SyncerError> {
        self.ensure_chain().await?;
        let from = *range.start();
        let to = *range.end();
        if from > to {
            return Ok(FetchResult::default());
        }

        let mut result = FetchResult::default();
        let mut batch_from = from;
        while batch_from <= to {
            let batch_to = to.min(
                batch_from
                    .saturating_add(self.max_block_range.get())
                    .saturating_sub(1),
            );
            let filter = Filter::new()
                .address(self.announcer)
                .event_signature(AnnouncementEvent::SIGNATURE_HASH)
                .topic1(B256::from(U256::from(SCHEME_ID)))
                .from_block(batch_from)
                .to_block(batch_to);
            let logs = self.provider.get_logs(&filter).await.map_err(rpc)?;
            let received = logs.len();
            for log in logs {
                if let Some(record) = decode_announcement(&log) {
                    result.announcements.push(record);
                } else {
                    result.rejected_logs += 1;
                    warn!(
                        block_number = log.block_number,
                        transaction_hash = ?log.transaction_hash,
                        log_index = log.log_index,
                        "rejected malformed scheme 3 announcement log"
                    );
                }
            }
            debug!(
                from_block = batch_from,
                to_block = batch_to,
                received,
                "fetched announcement logs"
            );
            if batch_to == to {
                break;
            }
            batch_from = batch_to + 1;
        }
        Ok(result)
    }
}

fn decode_announcement(log: &alloy::rpc::types::Log) -> Option<AnnouncementRecord> {
    if log.removed {
        return None;
    }
    let decoded = AnnouncementEvent::decode_log(&log.inner).ok()?;
    let scheme_id: u64 = decoded.data.schemeId.try_into().ok()?;
    if scheme_id != SCHEME_ID {
        return None;
    }
    let announcement = Announcement::from_parts(
        decoded.data.stealthAddress,
        decoded.data.ephemeralPubKey.to_vec(),
        decoded.data.metadata.to_vec(),
    )
    .ok()?;
    Some(AnnouncementRecord::new(
        &announcement,
        decoded.data.caller,
        log.block_number?,
        log.block_hash?,
        log.transaction_hash?,
        log.log_index?,
    ))
}
