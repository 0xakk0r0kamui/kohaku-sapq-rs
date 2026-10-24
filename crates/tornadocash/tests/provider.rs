use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use kohaku_fork_kit::pool::deploy_pool;
use kohaku_tornadocash::{
    Deposit, Withdrawal,
    merkle_tree::{MerkleTree, MerkleTreeExt},
    syncer::{Syncer, rpc::RpcSyncer},
};

#[tokio::test]
#[ignore = "run with `cargo test --release -- --ignored`"]
async fn test_deposit() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    let provider = ProviderBuilder::new().connect_anvil_with_wallet().erased();
    let pool = deploy_pool(provider.clone(), None).await?;

    // Deposit a note
    let deposit = Deposit::new(&pool, rand::random());
    provider
        .send_transaction(deposit.call().into())
        .await?
        .watch()
        .await?;

    // Assert recipient balance
    let balance = provider.get_balance(pool.address).await?;
    assert!(
        balance == U256::from(pool.amount_wei),
        "Pool balance should be non-zero"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "run with `cargo test --release -- --ignored`"]
async fn test_withdraw() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    let provider = ProviderBuilder::new().connect_anvil_with_wallet().erased();
    let pool = deploy_pool(provider.clone(), None).await?;

    // Deposit a note
    let deposit = Deposit::new(&pool, rand::random());
    let note = deposit.note.clone();
    provider
        .send_transaction(deposit.call().into())
        .await?
        .watch()
        .await?;

    // Construct a TornadoProvider
    let syncer = RpcSyncer::new(provider.clone());

    // Sync a Merkle tree against the provider
    let mut tree = MerkleTree::new();
    let snapshot = syncer.sync(&pool, ..).await?;
    tree.splice_events(&snapshot.events)?;

    // Withdraw the note
    let recipient: Address = PrivateKeySigner::random().address();
    let merkle_proof = tree.leaf_proof(note.commitment())?;
    let withdrawal =
        Withdrawal::new(&pool, note, recipient).prove(&merkle_proof, &mut rand::rng())?;
    provider
        .send_transaction(withdrawal.call().into())
        .await?
        .watch()
        .await?;

    // Assert recipient balance
    let balance = provider.get_balance(recipient).await?;
    assert!(
        balance == U256::from(pool.amount_wei),
        "Recipient balance should be non-zero"
    );

    Ok(())
}
