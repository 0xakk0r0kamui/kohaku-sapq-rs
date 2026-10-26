use alloy::primitives::B256;
use kohaku_tornadocash::{Pool, event::SyncEvent};
#[cfg(feature = "saga-sync")]
use kohaku_tornadocash_syncer::saga::SagaSyncer;
use kohaku_tornadocash_syncer::{Syncer, remote::RemoteSyncer};

const REMOTE_SYNC_BASE_URL: &str = "https://raw.githubusercontent.com/Robert-MacWha/privacy-protocols/refs/heads/sync-state/tornadocash-sync";
#[cfg(feature = "saga-sync")]
const SAGA_SYNC_BASE_URL: &str = "https://saga.fatsolutions.xyz/sepolia";

const FROM_BLOCK: u64 = Pool::SEPOLIA_ETHER_01.deployed_block;
const TO_BLOCK: u64 = 7_000_000;

#[tokio::test]
#[ignore = "run with `cargo test --release -- --ignored`"]
async fn test_remote_sync_matches_snapshot() -> Result<(), anyhow::Error> {
    assert_matches_snapshot(&RemoteSyncer::new(REMOTE_SYNC_BASE_URL)).await
}

#[tokio::test]
#[cfg(feature = "saga-sync")]
#[ignore = "run with `cargo test --release -- --ignored`"]
async fn test_saga_sync_matches_snapshot() -> Result<(), anyhow::Error> {
    assert_matches_snapshot(&SagaSyncer::new(SAGA_SYNC_BASE_URL)).await
}

/// Syncs `target_syncer` over [`FROM_BLOCK`, `TO_BLOCK`) and asserts its reported event log
/// exactly matches a checked-in snapshot.
async fn assert_matches_snapshot(target_syncer: &dyn Syncer) -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init()
        .ok();

    let pool = Pool::SEPOLIA_ETHER_01;
    let synced = target_syncer
        .sync_range(&pool, FROM_BLOCK, TO_BLOCK)
        .await?;

    let (mut commitments, mut nullifiers) = commitments_and_nullifiers(synced.events());
    commitments.sort();
    nullifiers.sort();

    insta::assert_debug_snapshot!("external_sync_events", (commitments, nullifiers));

    Ok(())
}

fn commitments_and_nullifiers(events: &[SyncEvent]) -> (Vec<B256>, Vec<B256>) {
    let mut commitments = Vec::new();
    let mut nullifiers = Vec::new();

    for event in events {
        match event {
            SyncEvent::Deposit(d) => commitments.push(d.commitment.into()),
            SyncEvent::Withdrawal(w) => nullifiers.push(w.nullifier_hash.into()),
        }
    }

    (commitments, nullifiers)
}
