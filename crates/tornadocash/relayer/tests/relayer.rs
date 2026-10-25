use alloy::{
    node_bindings::Anvil,
    primitives::U256,
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use kohaku_fork_kit::{
    pool::{deploy_pool, deploy_proxy},
    relayer::RelayerBuilder,
};
use kohaku_tornadocash::{
    Deposit, Withdrawal,
    merkle_tree::{MerkleTree, MerkleTreeExt},
};
use kohaku_tornadocash_syncer::{Syncer, rpc::RpcSyncer};

#[tokio::test]
#[ignore = "run with `cargo test --release -- --ignored`"]
async fn test_relayer_withdraw() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    let anvil = Anvil::new().block_time(1).try_spawn()?;
    let wallet = anvil.wallet().expect("anvil provides a dev wallet");
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(anvil.endpoint_url())
        .erased();

    let pool = deploy_pool(provider.clone(), None).await?;
    let proxy_address = deploy_proxy(provider.clone()).await?;

    let relayer_signer = PrivateKeySigner::random();
    let reward_account = PrivateKeySigner::random().address();
    let relayer = RelayerBuilder::new(
        anvil.endpoint(),
        anvil.ws_endpoint(),
        pool.clone(),
        proxy_address,
        relayer_signer.to_bytes().to_string(),
        reward_account,
    )
    .prefund(&provider)
    .await?
    .spawn()
    .await?;

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
    tree.splice_events(snapshot.events())?;

    // Withdraw the note via the relayer
    let recipient = PrivateKeySigner::random().address();
    let gas_price = provider.get_gas_price().await?;
    let payer = relayer.quote(&pool, gas_price, U256::ZERO).await?;
    let merkle_proof = tree.note_proof(&note)?;
    let withdrawal = Withdrawal::new(&pool, note, recipient)
        .with_payer(payer)
        .prove(&merkle_proof, &mut rand::rng())?;

    let receipt = relayer.withdraw(withdrawal).await?;
    let tx_hash = relayer
        .await_confirmation(&provider, &receipt)
        .await?
        .unwrap();

    // Assert transaction was successful
    let tx_receipt = provider.get_transaction_receipt(tx_hash).await?;
    assert!(tx_receipt.is_some(), "Transaction receipt should exist");
    let tx_receipt = tx_receipt.unwrap();
    assert!(tx_receipt.status(), "Transaction should succeed");

    // Assert recipient balance
    let balance = provider.get_balance(recipient).await?;
    assert!(
        balance > U256::ZERO,
        "Recipient balance should be non-zero after relayer withdrawal"
    );

    Ok(())
}
