use alloy::{providers::Provider, rpc::types::Filter};
use kohaku_rpc_provider::{StreamedEvent, StreamedEventConfig};
use kohaku_tornadocash::{
    Pool,
    event::{Snapshot, SyncEvent},
};
use tracing::{info, warn};

use crate::{Syncer, SyncerError};

/// A syncer that reads from an Ethereum JSON-RPC provider
#[derive(Clone)]
pub struct RpcSyncer<P: Provider> {
    provider: P,
    config: StreamedEventConfig,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
enum RpcSyncerError {
    #[error("Error decoding log: {0}")]
    LogDecodeError(#[from] alloy::sol_types::Error),
    #[error("RPC error: {0}")]
    RpcError(#[from] alloy::transports::RpcError<alloy::transports::TransportErrorKind>),
    #[error("Field conversion error: {0}")]
    Field(#[from] kohaku_tornadocash::FieldNotInRangeError),
    #[error("Snapshot error: {0}")]
    Snapshot(#[from] kohaku_tornadocash::event::SnapshotError),
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
            return Ok(Snapshot::new(pool.clone(), range, Vec::new())?);
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
        Ok(Snapshot::new(pool.clone(), range, events)?)
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

#[cfg(test)]
mod tests {
    use alloy::{
        primitives::{Address, B256, LogData, U256},
        providers::ProviderBuilder,
        rpc::types::Log,
        sol,
        sol_types::SolEvent,
        transports::mock::Asserter,
    };
    use kohaku_tornadocash::event::{DepositEvent, WithdrawalEvent};

    use super::*;

    sol!(
        event Deposit(bytes32 indexed commitment, uint32 leafIndex, uint256 timestamp);
        event Withdrawal(address to, bytes32 nullifierHash, address indexed relayer, uint256 fee);
    );

    #[tokio::test]
    async fn syncs_events_within_clamped_range() {
        let pool = Pool::SEPOLIA_ETHER_01;
        let deposit_block = pool.deployed_block + 3;
        let latest = pool.deployed_block + 9;
        let commitment = B256::with_last_byte(1);
        let nullifier_hash = B256::with_last_byte(2);
        let to = Address::with_last_byte(3);
        let relayer = Address::with_last_byte(4);
        let fee = U256::from(5);

        let deposit = Deposit {
            commitment,
            leafIndex: 0,
            timestamp: U256::ZERO,
        };
        let withdrawal = Withdrawal {
            to,
            nullifierHash: nullifier_hash,
            relayer,
            fee,
        };

        let asserter = Asserter::new();
        asserter.push_success(&latest);
        asserter.push_success(&vec![
            log_at(&pool, deposit.encode_log_data(), deposit_block),
            log_at(&pool, withdrawal.encode_log_data(), latest),
        ]);
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        let snapshot = RpcSyncer::new(provider)
            .sync_range(&pool, 0, u64::MAX)
            .await
            .unwrap();

        assert_eq!(snapshot.range(), pool.deployed_block..latest + 1);
        assert_eq!(
            snapshot.events(),
            &[
                SyncEvent::Deposit(DepositEvent {
                    commitment: commitment.try_into().unwrap(),
                    leaf_index: 0,
                    block_number: deposit_block,
                }),
                SyncEvent::Withdrawal(WithdrawalEvent {
                    to,
                    nullifier_hash: nullifier_hash.try_into().unwrap(),
                    relayer,
                    fee,
                    block_number: latest,
                }),
            ]
        );
        assert!(asserter.read_q().is_empty());
    }

    fn log_at(pool: &Pool, data: LogData, block: u64) -> Log {
        Log {
            inner: alloy::primitives::Log {
                address: pool.address,
                data,
            },
            block_number: Some(block),
            ..Default::default()
        }
    }
}
