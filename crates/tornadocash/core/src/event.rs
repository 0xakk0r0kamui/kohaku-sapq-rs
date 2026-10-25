use std::ops::Range;

use alloy::{
    primitives::{Address, U256},
    rpc::types::Log,
    sol_types::SolEvent,
};
use serde::{Deserialize, Serialize};

use crate::{abis::tornado::Tornado, field::Field, pool::Pool};

/// A set of events emitted by a pool within a given block range.
///
/// Every event lies within the snapshot's range. The snapshot's deposits are contiguous
/// in ascending order by leaf index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSnapshot")]
pub struct Snapshot {
    pool: Pool,
    range: Range<u64>,
    events: Vec<SyncEvent>,
}

/// Unvalidated [`Snapshot`].
#[derive(Deserialize)]
struct RawSnapshot {
    pool: Pool,
    range: Range<u64>,
    events: Vec<SyncEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncEvent {
    Deposit(DepositEvent),
    Withdrawal(WithdrawalEvent),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositEvent {
    pub commitment: Field,
    pub leaf_index: u32,
    pub block_number: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawalEvent {
    pub to: Address,
    pub nullifier_hash: Field,
    pub relayer: Address,
    pub fee: U256,
    pub block_number: u64,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotError {
    #[error("Event at block {block} outside snapshot range {range:?}")]
    EventOutOfRange { block: u64, range: Range<u64> },
    #[error("Non-contiguous deposits: leaf {actual} follows leaf {previous}")]
    NonContiguousDeposits { previous: u32, actual: u32 },
}

impl Snapshot {
    /// Creates a snapshot of `pool`'s `events` within the half-open block `range`.
    ///
    /// # Errors
    /// Returns an error if an event lies outside `range`, or if the deposits are not contiguous
    /// and in ascending order by leaf index.
    pub fn new(
        pool: Pool,
        range: Range<u64>,
        events: Vec<SyncEvent>,
    ) -> Result<Self, SnapshotError> {
        if let Some(block) = events
            .iter()
            .map(SyncEvent::block_number)
            .find(|block| !range.contains(block))
        {
            return Err(SnapshotError::EventOutOfRange { block, range });
        }

        let leaves: Vec<u32> = events.iter().filter_map(SyncEvent::leaf_index).collect();
        if let Some(pair) = leaves
            .windows(2)
            .find(|pair| pair[0].checked_add(1) != Some(pair[1]))
        {
            return Err(SnapshotError::NonContiguousDeposits {
                previous: pair[0],
                actual: pair[1],
            });
        }

        Ok(Self {
            pool,
            range,
            events,
        })
    }

    #[must_use]
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    #[must_use]
    pub fn range(&self) -> Range<u64> {
        self.range.clone()
    }

    #[must_use]
    pub fn events(&self) -> &[SyncEvent] {
        &self.events
    }
}

impl SyncEvent {
    /// Attempts to decode an alloy RPC log into a `SyncEvent`.
    ///
    /// Returns `None` if the log is not a known event.
    pub fn try_from_log(log: &Log) -> Option<Self> {
        match log.topics().first() {
            Some(&Tornado::Deposit::SIGNATURE_HASH) => {
                let decoded = Tornado::Deposit::decode_log(&log.inner).ok()?.data;
                Some(SyncEvent::Deposit(DepositEvent {
                    commitment: decoded.commitment.try_into().ok()?,
                    leaf_index: decoded.leafIndex,
                    block_number: log.block_number?,
                }))
            }
            Some(&Tornado::Withdrawal::SIGNATURE_HASH) => {
                let decoded = Tornado::Withdrawal::decode_log(&log.inner).ok()?.data;
                Some(SyncEvent::Withdrawal(WithdrawalEvent {
                    to: decoded.to,
                    nullifier_hash: decoded.nullifierHash.try_into().ok()?,
                    relayer: decoded.relayer,
                    fee: decoded.fee,
                    block_number: log.block_number?,
                }))
            }
            _ => None,
        }
    }

    fn block_number(&self) -> u64 {
        match self {
            SyncEvent::Deposit(deposit) => deposit.block_number,
            SyncEvent::Withdrawal(withdrawal) => withdrawal.block_number,
        }
    }

    fn leaf_index(&self) -> Option<u32> {
        match self {
            SyncEvent::Deposit(deposit) => Some(deposit.leaf_index),
            SyncEvent::Withdrawal(_) => None,
        }
    }
}

impl TryFrom<RawSnapshot> for Snapshot {
    type Error = SnapshotError;

    fn try_from(raw: RawSnapshot) -> Result<Self, Self::Error> {
        Snapshot::new(raw.pool, raw.range, raw.events)
    }
}
