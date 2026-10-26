use alloy::primitives::{Address, B256, U256};
use kohaku_tornadocash::{
    Pool,
    event::{DepositEvent, Snapshot, SyncEvent, WithdrawalEvent},
};
use reqwest::Client;
use serde::Deserialize;
use tracing::info;

use crate::{Syncer, SyncerError};

/// A syncer that reads from a remote database of cached data.
///
/// The remote database is expected to point to a directory of cache files.
/// The cache files should be named in the format
/// `{chain_id}_{asset}_{amount}_{deposits/nullifiers}.ndjson` and should contain
/// newline-delimited JSON objects representing deposits and nullifiers for the
/// given pool.
pub struct RemoteSyncer {
    client: Client,
    base_url: String,
}

#[derive(Deserialize)]
struct RemoteDeposit {
    pub block_number: u64,
    pub commitment: B256,
    pub leaf_index: u32,
    // pub timestamp: U256,
}

#[derive(Deserialize)]
struct RemoteWithdrawal {
    pub block_number: u64,
    pub nullifier: B256,
    pub to: Address,
    pub fee: U256,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
enum RemoteSyncerError {
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("Serde error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Field conversion error: {0}")]
    Field(#[from] kohaku_tornadocash::FieldNotInRangeError),
    #[error("Snapshot error: {0}")]
    Snapshot(#[from] kohaku_tornadocash::event::SnapshotError),
}

impl RemoteSyncer {
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.to_string(),
        }
    }
}

#[cfg_attr(native, async_trait::async_trait)]
#[cfg_attr(wasm, async_trait::async_trait(?Send))]
impl Syncer for RemoteSyncer {
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

impl RemoteSyncer {
    async fn sync(
        &self,
        pool: &Pool,
        from_block: u64,
        to_block: u64,
    ) -> Result<Snapshot, RemoteSyncerError> {
        let deposits = self.deposits(pool).await?;
        let withdrawals = self.withdrawals(pool).await?;

        //? The cache only covers up to the last block it holds an event for.
        let from = from_block.max(pool.deployed_block);
        let latest = latest_block(&deposits, &withdrawals).saturating_add(1);
        let range = from..to_block.min(latest).max(from);

        info!("Syncing from {} to {}", range.start, range.end);

        let deposits: Vec<SyncEvent> = deposits
            .iter()
            .filter(|d| range.contains(&d.block_number))
            .map(decode_deposit)
            .collect::<Result<_, _>>()?;
        let withdrawals: Vec<SyncEvent> = withdrawals
            .iter()
            .filter(|n| range.contains(&n.block_number))
            .map(decode_withdrawal)
            .collect::<Result<_, _>>()?;

        let events = deposits.into_iter().chain(withdrawals).collect();

        Ok(Snapshot::new(pool.clone(), range, events)?)
    }

    async fn deposits(&self, pool: &Pool) -> Result<Vec<RemoteDeposit>, RemoteSyncerError> {
        let resp = self
            .client
            .get(deposits_url(&self.base_url, pool))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let deposits = resp
            .lines()
            .filter(|line| !line.is_empty())
            .map(serde_json::from_str::<RemoteDeposit>)
            .collect::<Result<_, _>>()?;
        Ok(deposits)
    }

    async fn withdrawals(&self, pool: &Pool) -> Result<Vec<RemoteWithdrawal>, RemoteSyncerError> {
        let withdrawals = self
            .client
            .get(withdrawals_url(&self.base_url, pool))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let withdrawals = withdrawals
            .lines()
            .filter(|line| !line.is_empty())
            .map(serde_json::from_str::<RemoteWithdrawal>)
            .collect::<Result<_, _>>()?;
        Ok(withdrawals)
    }
}

fn decode_deposit(deposit: &RemoteDeposit) -> Result<SyncEvent, RemoteSyncerError> {
    Ok(SyncEvent::Deposit(DepositEvent {
        commitment: deposit.commitment.try_into()?,
        leaf_index: deposit.leaf_index,
        block_number: deposit.block_number,
    }))
}

fn decode_withdrawal(withdrawal: &RemoteWithdrawal) -> Result<SyncEvent, RemoteSyncerError> {
    Ok(SyncEvent::Withdrawal(WithdrawalEvent {
        to: withdrawal.to,
        nullifier_hash: withdrawal.nullifier.try_into()?,
        relayer: Address::ZERO,
        fee: withdrawal.fee,
        block_number: withdrawal.block_number,
    }))
}

/// Returns the highest block number covered by the cached deposits and withdrawals.
fn latest_block(deposits: &[RemoteDeposit], withdrawals: &[RemoteWithdrawal]) -> u64 {
    let latest_deposit = deposits.iter().map(|d| d.block_number).max().unwrap_or(0);
    let latest_withdrawal = withdrawals
        .iter()
        .map(|n| n.block_number)
        .max()
        .unwrap_or(0);

    latest_deposit.max(latest_withdrawal)
}

fn deposits_url(base: &str, pool: &Pool) -> String {
    url(base, pool, "deposits")
}

fn withdrawals_url(base: &str, pool: &Pool) -> String {
    url(base, pool, "nullifiers")
}

fn url(base: &str, pool: &Pool, suffix: &str) -> String {
    format!(
        "{}/{}_{}_{}_{}.ndjson",
        base,
        pool.chain_id,
        pool.symbol().to_uppercase(),
        pool.amount(),
        suffix
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deposits_url() {
        let pool = Pool::ETHEREUM_ETHER_01;
        let url = deposits_url("http://example.com", &pool);
        assert_eq!(url.as_str(), "http://example.com/1_ETH_0.1_deposits.ndjson");
    }

    #[test]
    fn test_withdrawals_url() {
        let pool = Pool::ETHEREUM_ETHER_01;
        let url = withdrawals_url("http://example.com", &pool);
        assert_eq!(
            url.as_str(),
            "http://example.com/1_ETH_0.1_nullifiers.ndjson"
        );
    }
}
