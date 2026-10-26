use std::collections::HashMap;

use alloy::signers::Signer;
use kohaku_tornadocash::{
    Field, Pool,
    event::{DepositEvent, Snapshot, SyncEvent, WithdrawalEvent},
};

use crate::{Keychain, KeychainError};

/// Number of consecutive nonces without a deposit after which a scan gives up.
const DEFAULT_GAP_LIMIT: u64 = 20;

/// A note recovered from a pool's synced events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredNote {
    pub pool: Pool,
    /// The nonce used to derive the note.
    pub nonce: u64,
    /// The deposit event that created the note.
    pub deposit: DepositEvent,
    /// The withdrawal event that spent the note, if any.
    pub withdrawal: Option<WithdrawalEvent>,
}

/// One pool's events, indexed by commitment and nullifier hash.
struct PoolIndex<'a> {
    pool: &'a Pool,
    deposits: HashMap<Field, &'a DepositEvent>,
    withdrawals: HashMap<Field, &'a WithdrawalEvent>,
}

/// Returns the next nonce that should be used for a new note derived from `keychain`.
///
/// Nonces are shared across all pools, so this is one past the highest nonce recovered in any
/// pool. See [`recover`] for details on how notes are recovered.
pub async fn next_nonce<S: Signer + Send + Sync>(
    keychain: &Keychain<S>,
    batches: &[Snapshot],
    gap_limit: Option<u64>,
) -> Result<u64, KeychainError> {
    let recovered = recover(keychain, batches, gap_limit).await?;
    Ok(recovered.last().map_or(0, |note| note.nonce + 1))
}

/// Recovers every note `keychain` derived for the pools in `batches`.
///
/// Incrementally scans nonces starting from 0, finding deposits and withdrawals in `batches` that
/// match each derived note. Scanning stops after `gap_limit` consecutive nonces with no deposits
/// in any pool.
///
/// Each nonce is checked against every pool, so a nonce that was reused across pools recovers a
/// note for each of them. Batches for the same pool are combined, so a pool's events may be split
/// across several block ranges.
///
/// Recovery may be incomplete if `batches` do not contain all deposits and withdrawals made by
/// `keychain` for the pools.
///
/// # Errors
/// Returns an error if the keychain cannot derive a nonce's material.
pub async fn recover<S: Signer + Send + Sync>(
    keychain: &Keychain<S>,
    batches: &[Snapshot],
    gap_limit: Option<u64>,
) -> Result<Vec<RecoveredNote>, KeychainError> {
    let gap_limit = gap_limit.unwrap_or(DEFAULT_GAP_LIMIT);
    if gap_limit == 0 {
        return Ok(Vec::new());
    }

    let indexes = index(batches);
    let pools: Vec<Pool> = indexes.iter().map(|index| index.pool.clone()).collect();

    let mut recovered = Vec::new();
    let mut misses = 0;
    for nonce in 0.. {
        if misses >= gap_limit {
            break;
        }
        misses += 1;

        let hashes = keychain.note_hashes(nonce, &pools).await?;
        for (index, hashes) in indexes.iter().zip(&hashes) {
            let Some(deposit) = index.deposits.get(&hashes.commitment).copied().cloned() else {
                continue;
            };

            misses = 0;
            let withdrawal = index
                .withdrawals
                .get(&hashes.nullifier_hash)
                .copied()
                .cloned();

            recovered.push(RecoveredNote {
                pool: index.pool.clone(),
                nonce,
                deposit,
                withdrawal,
            });
        }
    }

    Ok(recovered)
}

/// Indexes the snapshots in `batches` by pool, combining batches that share a pool.
fn index(batches: &[Snapshot]) -> Vec<PoolIndex<'_>> {
    let mut indexes: Vec<PoolIndex> = Vec::new();
    for batch in batches {
        let deposits = deposits(batch.events());
        let withdrawals = withdrawals(batch.events());

        if let Some(index) = indexes.iter_mut().find(|index| index.pool == batch.pool()) {
            index.deposits.extend(deposits);
            index.withdrawals.extend(withdrawals);
            continue;
        }

        indexes.push(PoolIndex {
            pool: batch.pool(),
            deposits,
            withdrawals,
        });
    }
    indexes
}

/// Indexes the deposits in `events` by commitment.
fn deposits(events: &[SyncEvent]) -> HashMap<Field, &DepositEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            SyncEvent::Deposit(deposit) => Some((deposit.commitment, deposit)),
            SyncEvent::Withdrawal(_) => None,
        })
        .collect()
}

/// Indexes the withdrawals in `events` by nullifier hash.
fn withdrawals(events: &[SyncEvent]) -> HashMap<Field, &WithdrawalEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            SyncEvent::Withdrawal(withdrawal) => Some((withdrawal.nullifier_hash, withdrawal)),
            SyncEvent::Deposit(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use alloy::{
        primitives::{Address, U256},
        signers::local::PrivateKeySigner,
    };
    use kohaku_tornadocash::Note;

    use super::*;

    const POOL: Pool = Pool::ETHEREUM_ETHER_01;
    const POOL2: Pool = Pool::ETHEREUM_ETHER_10;

    #[tokio::test]
    async fn recovers_deposits() {
        let batches = vec![batch(
            &POOL,
            vec![
                deposit(&note(&POOL, 0).await, 0),
                deposit(&note(&POOL2, 1).await, 1),
                deposit(&note(&POOL, 1).await, 2),
            ],
        )];

        let recovered = recover(&keychain(), &batches, Some(2)).await.unwrap();

        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].nonce, 0);
        assert_eq!(recovered[0].deposit.leaf_index, 0);
        assert_eq!(recovered[1].nonce, 1);
        assert_eq!(recovered[1].deposit.leaf_index, 2);
    }

    #[tokio::test]
    async fn recovers_deposits_from_multiple_pools() {
        let batches = vec![
            batch(
                &POOL,
                vec![
                    deposit(&note(&POOL, 0).await, 0),
                    deposit(&note(&POOL, 2).await, 1),
                ],
            ),
            batch(&POOL2, vec![deposit(&note(&POOL2, 1).await, 0)]),
        ];

        let recovered = recover(&keychain(), &batches, Some(2)).await.unwrap();

        assert_eq!(recovered.len(), 3);
        assert_eq!((recovered[0].pool.clone(), recovered[0].nonce), (POOL, 0));
        assert_eq!((recovered[1].pool.clone(), recovered[1].nonce), (POOL2, 1));
        assert_eq!((recovered[2].pool.clone(), recovered[2].nonce), (POOL, 2));
    }

    #[tokio::test]
    async fn recovers_withdrawals() {
        let batches = vec![batch(
            &POOL,
            vec![
                deposit(&note(&POOL, 0).await, 0),
                deposit(&note(&POOL, 1).await, 1),
                withdrawal(&note(&POOL, 1).await),
                withdrawal(&note(&POOL2, 0).await),
            ],
        )];

        let recovered = recover(&keychain(), &batches, Some(2)).await.unwrap();

        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].nonce, 0);
        assert_eq!(recovered[0].withdrawal, None);
        assert_eq!(recovered[1].nonce, 1);
        assert!(recovered[1].withdrawal.is_some());
    }

    #[tokio::test]
    async fn recovers_withdrawal_from_separate_batch() {
        let batches = vec![
            batch(&POOL, vec![deposit(&note(&POOL, 0).await, 0)]),
            batch(&POOL, vec![withdrawal(&note(&POOL, 0).await)]),
        ];

        let recovered = recover(&keychain(), &batches, Some(2)).await.unwrap();

        assert_eq!(recovered.len(), 1);
        assert!(recovered[0].withdrawal.is_some());
    }

    #[tokio::test]
    async fn stops_at_gap_limit() {
        let batches = vec![batch(
            &POOL,
            vec![
                deposit(&note(&POOL, 0).await, 0),
                deposit(&note(&POOL, 3).await, 1),
            ],
        )];

        let recovered = recover(&keychain(), &batches, Some(2)).await.unwrap();

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].nonce, 0);
    }

    #[tokio::test]
    async fn next_nonce_is_incremented() {
        let batches = vec![
            batch(
                &POOL,
                vec![
                    deposit(&note(&POOL, 0).await, 0),
                    withdrawal(&note(&POOL, 0).await),
                ],
            ),
            batch(&POOL2, vec![deposit(&note(&POOL2, 1).await, 0)]),
        ];

        let next_nonce = next_nonce(&keychain(), &batches, None).await.unwrap();
        assert_eq!(next_nonce, 2);
    }

    fn keychain() -> Keychain<PrivateKeySigner> {
        Keychain::new(PrivateKeySigner::from_bytes(&U256::from(42).into()).unwrap())
    }

    async fn note(pool: &Pool, nonce: u64) -> Note {
        keychain().note(nonce, pool).await.unwrap().note
    }

    fn batch(pool: &Pool, events: Vec<SyncEvent>) -> Snapshot {
        Snapshot::new(pool.clone(), 0..1, events).unwrap()
    }

    fn deposit(note: &Note, leaf_index: u32) -> SyncEvent {
        SyncEvent::Deposit(DepositEvent {
            commitment: note.commitment(),
            leaf_index,
            block_number: 0,
        })
    }

    fn withdrawal(note: &Note) -> SyncEvent {
        SyncEvent::Withdrawal(WithdrawalEvent {
            to: Address::ZERO,
            nullifier_hash: note.nullifier_hash(),
            relayer: Address::ZERO,
            fee: U256::ZERO,
            block_number: 0,
        })
    }
}
