use crate::scheme;

use alloy::{
    network::TransactionBuilder,
    primitives::{Address, B256, U256},
    providers::{DynProvider, Provider},
    rpc::types::{Filter, TransactionRequest},
    sol,
    sol_types::{SolCall, SolEvent},
};
use kohaku_kv_store::{Store, backend::StoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

sol! {
    interface IERC5564Announcer {
        event Announcement(
            uint256 indexed schemeId,
            address indexed stealthAddress,
            address indexed caller,
            bytes ephemeralPubKey,
            bytes metadata
        );
        function announce(
            uint256 schemeId,
            address stealthAddress,
            bytes calldata ephemeralPubKey,
            bytes calldata metadata
        ) external;
    }

    interface IERC6538Registry {
        function registerKeys(uint256 schemeId, bytes calldata stealthMetaAddress) external;
        function stealthMetaAddressOf(address registrant, uint256 schemeId)
            external
            view
            returns (bytes memory stealthMetaAddress);
    }

    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
    }
}

const REORG_MARGIN: u64 = 32;
const LOG_BATCH: u64 = 10_000;

const CURSOR_KEY: &[u8] = b"cursor";
const RECORDS_KEY: &[u8] = b"records";

#[derive(Clone, Copy, Debug)]
pub struct Deployment {
    pub announcer: Address,
    pub registry: Address,
    pub start_block: u64,
    pub scheme_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncReport {
    pub from_block: u64,
    pub to_block: u64,
    pub stored: usize,
    pub undecoded: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub scheme_id: u64,
    pub stealth_address: [u8; 20],
    pub caller: [u8; 20],
    pub ephemeral_pub_key: Vec<u8>,
    pub metadata: Vec<u8>,
    pub block_number: u64,
    pub log_index: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asset {
    Native,
    Erc20(Address),
}

#[derive(Debug, Error)]
pub enum ChainError {
    #[error("rpc call failed")]
    Rpc(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("stored announcement log could not be decoded")]
    CorruptStore(#[source] postcard::Error),
    #[error("registry return data could not be decoded")]
    BadRegistryReturn,
}

fn rpc(err: impl std::error::Error + Send + Sync + 'static) -> ChainError {
    ChainError::Rpc(Box::new(err))
}

#[derive(Clone)]
pub struct Chain {
    store: Store,
    provider: DynProvider,
    deployment: Deployment,
    sync_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl Chain {
    #[must_use]
    pub fn new(store: &Store, provider: DynProvider, deployment: Deployment) -> Self {
        Self {
            store: store.clone(),
            provider,
            deployment,
            sync_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    async fn scoped(&self) -> Result<Store, ChainError> {
        let chain_id = self.provider.get_chain_id().await.map_err(rpc)?;
        Ok(self.store.scope(format!(
            "kohaku-stealth/logs/{chain_id}/{:x}/{}",
            self.deployment.announcer, self.deployment.scheme_id
        )))
    }

    pub fn register_keys_tx(&self, meta_address: &[u8]) -> TransactionRequest {
        let call = IERC6538Registry::registerKeysCall {
            schemeId: U256::from(self.deployment.scheme_id),
            stealthMetaAddress: meta_address.to_vec().into(),
        };
        TransactionRequest::default()
            .with_to(self.deployment.registry)
            .with_input(call.abi_encode())
    }

    pub fn announce_tx(
        &self,
        stealth_address: Address,
        ephemeral_pub_key: &[u8],
        metadata: &[u8],
    ) -> TransactionRequest {
        let call = IERC5564Announcer::announceCall {
            schemeId: U256::from(self.deployment.scheme_id),
            stealthAddress: stealth_address,
            ephemeralPubKey: ephemeral_pub_key.to_vec().into(),
            metadata: metadata.to_vec().into(),
        };
        TransactionRequest::default()
            .with_to(self.deployment.announcer)
            .with_input(call.abi_encode())
    }

    #[must_use]
    pub fn fund_tx(asset: Asset, to: Address, amount: U256) -> TransactionRequest {
        match asset {
            Asset::Native => TransactionRequest::default().with_to(to).with_value(amount),
            Asset::Erc20(token) => {
                let call = IERC20::transferCall { to, amount };
                TransactionRequest::default()
                    .with_to(token)
                    .with_input(call.abi_encode())
            }
        }
    }

    /// # Errors
    ///
    /// RPC failure, or return data that is not `bytes`.
    pub async fn meta_address_of(&self, registrant: Address) -> Result<Vec<u8>, ChainError> {
        let call = IERC6538Registry::stealthMetaAddressOfCall {
            registrant,
            schemeId: U256::from(self.deployment.scheme_id),
        };
        let output = self
            .provider
            .call(
                TransactionRequest::default()
                    .with_to(self.deployment.registry)
                    .with_input(call.abi_encode()),
            )
            .await
            .map_err(rpc)?;
        let decoded = IERC6538Registry::stealthMetaAddressOfCall::abi_decode_returns(&output)
            .map_err(|_| ChainError::BadRegistryReturn)?;
        Ok(decoded.to_vec())
    }

    /// # Errors
    ///
    /// The store record is not the postcard encoding this crate wrote.
    pub async fn records(&self) -> Result<Vec<Record>, ChainError> {
        load_records(&self.scoped().await?).await
    }


    /// # Errors
    ///
    /// RPC or store failure. An undecodable log, or one whose field lengths are not
    /// scheme 3, is counted in [`SyncReport::undecoded`] and not stored.
    pub async fn sync(&self) -> Result<SyncReport, ChainError> {
        let _guard = self.sync_lock.lock().await;
        let store = self.scoped().await?;
        let latest = self.provider.get_block_number().await.map_err(rpc)?;
        let cursor = load_cursor(&store).await?;
        let from = match cursor {
            Some(saved) if saved <= latest => saved.saturating_sub(REORG_MARGIN),
            Some(_) => latest.saturating_sub(REORG_MARGIN),
            None => self.deployment.start_block,
        }
        .max(self.deployment.start_block);
        if from > latest {
            let mut records = load_records(&store).await?;
            records.retain(|record| record.block_number <= latest);
            save_state(&store, &records, latest).await?;
            info!(
                from_block = from,
                to_block = latest,
                stored = records.len(),
                "announcement sync dropped a future tip"
            );
            return Ok(SyncReport {
                from_block: from,
                to_block: latest,
                stored: records.len(),
                undecoded: 0,
            });
        }

        let mut fetched = Vec::new();
        let mut undecoded = 0usize;
        let mut batch_from = from;
        while batch_from <= latest {
            let batch_to = latest.min(batch_from.saturating_add(LOG_BATCH - 1));
            let filter = Filter::new()
                .address(self.deployment.announcer)
                .event_signature(IERC5564Announcer::Announcement::SIGNATURE_HASH)
                .topic1(B256::from(U256::from(self.deployment.scheme_id)))
                .from_block(batch_from)
                .to_block(batch_to);
            let logs = self.provider.get_logs(&filter).await.map_err(rpc)?;
            for log in logs {
                if let Some(record) = decode_announcement(&log) {
                    fetched.push(record);
                } else {
                    undecoded += 1;
                    warn!(block = log.block_number, "announcement log did not decode");
                }
            }
            if batch_to == latest {
                break;
            }
            batch_from = batch_to + 1;
        }

        let mut records = load_records(&store).await?;
        records.retain(|record| record.block_number < from);
        records.extend(fetched);
        let stored = records.len();
        save_state(&store, &records, latest).await?;
        info!(
            from_block = from,
            to_block = latest,
            stored,
            undecoded,
            scheme_id = self.deployment.scheme_id,
            "announcement sync"
        );
        Ok(SyncReport {
            from_block: from,
            to_block: latest,
            stored,
            undecoded,
        })
    }
}

fn decode_announcement(log: &alloy::rpc::types::Log) -> Option<Record> {
    let decoded = IERC5564Announcer::Announcement::decode_log(&log.inner).ok()?;
    Some(Record {
        scheme_id: decoded.data.schemeId.try_into().ok()?,
        stealth_address: decoded.data.stealthAddress.into(),
        caller: decoded.data.caller.into(),
        ephemeral_pub_key: decoded.data.ephemeralPubKey.to_vec(),
        metadata: decoded.data.metadata.to_vec(),
        block_number: log.block_number?,
        log_index: log.log_index?,
    })
    .filter(|record| scheme::announcement_shape_ok(&record.ephemeral_pub_key, &record.metadata))
}

async fn load_cursor(store: &Store) -> Result<Option<u64>, ChainError> {
    let Some(bytes) = store.get(CURSOR_KEY).await? else {
        return Ok(None);
    };
    postcard::from_bytes(&bytes).map_err(ChainError::CorruptStore)
}

async fn load_records(store: &Store) -> Result<Vec<Record>, ChainError> {
    let Some(bytes) = store.get(RECORDS_KEY).await? else {
        return Ok(Vec::new());
    };
    postcard::from_bytes(&bytes).map_err(ChainError::CorruptStore)
}

async fn save_state(store: &Store, records: &[Record], cursor: u64) -> Result<(), ChainError> {
    let records_bytes = postcard::to_allocvec(records).map_err(ChainError::CorruptStore)?;
    let cursor_bytes = postcard::to_allocvec(&cursor).map_err(ChainError::CorruptStore)?;
    store
        .put_batch([
            (RECORDS_KEY, records_bytes.as_slice()),
            (CURSOR_KEY, cursor_bytes.as_slice()),
        ])
        .await?;
    Ok(())
}
