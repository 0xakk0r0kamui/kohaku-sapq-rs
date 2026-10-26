use alloy::{
    node_bindings::Anvil,
    primitives::{Address, U256, address},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use kohaku_fork_kit::{
    alto::AltoBuilder,
    entry_point::deploy_entry_point,
    paymaster::{deploy_fee_adapter, deploy_paymaster},
    pool::deploy_pool,
    simple_account::deploy_simple_account,
};
use kohaku_tornadocash::{
    Deposit, PaymasterInfo, TornadoProviderExt, Withdrawal,
    merkle_tree::{MerkleTree, MerkleTreeExt},
};
use kohaku_tornadocash_paymaster::UserOperationPaymasterExt;
use kohaku_tornadocash_syncer::{Syncer, rpc::RpcSyncer};
use kohaku_userop_kit::{
    builder::UserOperationBuilder,
    bundler::Bundler,
    smart_account::simple_7702_smart_account::{Call, Simple7702SmartAccount},
};
use tracing::info;

const ALTO_EXECUTOR_PK: &str = "0x4a3a02862ddcb260ed52d40ef03f8e3d78fa3d174b0ef333afdf1ffb4a648cd5";
const ALTO_UTILITY_PK: &str = "0xdd4b2564c83ff7de602c39ffda1146055dc1814b07c083d7971722384f1f01a6";

const SINK: Address = address!("0x000000000000000000000000000000000000dead");
const PLACEHOLDER_FACTORY: Address = address!("0x0000000000000000000000000000000000000011");
const PLACEHOLDER_WETH: Address = address!("0x0000000000000000000000000000000000000022");

#[tokio::test]
#[ignore = "run with `cargo test --release -- --ignored`"]
async fn test_tornadocash_paymaster() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    let anvil = Anvil::new().try_spawn()?;
    let wallet = anvil.wallet().expect("anvil provides a dev wallet");
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(anvil.endpoint_url())
        .erased();
    let chain_id = provider.get_chain_id().await?;

    let entrypoint = deploy_entry_point(&provider).await?;
    deploy_simple_account(&provider).await?;

    let mut pool = deploy_pool(provider.clone(), None).await?;
    let paymaster_address = deploy_paymaster(
        provider.clone(),
        entrypoint,
        PLACEHOLDER_FACTORY,
        PLACEHOLDER_WETH,
    )
    .await?;
    let adapter_address =
        deploy_fee_adapter(provider.clone(), paymaster_address, pool.address).await?;
    pool.paymaster = Some(PaymasterInfo {
        address: paymaster_address,
        adapter: adapter_address,
    });

    // Deposit a note
    info!("Depositing into pool");
    let deposit = Deposit::new(&pool, rand::random());
    let note = deposit.note.clone();
    provider
        .send_transaction(deposit.call().into())
        .await?
        .watch()
        .await?;

    // Sync a Merkle tree against the provider
    let syncer = RpcSyncer::new(provider.clone());
    let mut tree = MerkleTree::new();
    let snapshot = syncer.sync(&pool, ..).await?;
    tree.splice_events(&snapshot)?;

    info!("Starting local alto bundler");
    let alto = AltoBuilder::new(
        anvil.endpoint(),
        entrypoint,
        ALTO_EXECUTOR_PK,
        ALTO_UTILITY_PK,
    )
    .prefund(&provider)
    .await?
    .spawn()
    .await?;

    // Sponsor a UserOp with the note
    info!("Withdrawing from pool with tornadocash paymaster");
    let owner = PrivateKeySigner::random();
    let smart_account = Simple7702SmartAccount::new(provider.clone(), owner.address(), chain_id);

    let builder = UserOperationBuilder::new_with_smart_account(&smart_account)
        .await?
        .with_call(&vec![Call {
            target: SINK,
            ..Default::default()
        }]);

    let withdrawal = Withdrawal::new(&pool, note.clone(), owner.address());
    let withdrawal_merkle_proof = tree.note_proof(&withdrawal.note)?;
    let builder = builder
        .with_tornado_paymaster(
            withdrawal,
            &withdrawal_merkle_proof,
            &provider,
            &*alto,
            &mut rand::rng(),
        )
        .await?;

    let userop = builder.build().sign(&owner).await?;
    let userop_hash = alto.send_user_operation(&userop).await?;
    let userop_receipt = alto.wait_for_receipt(userop_hash).await?;
    info!("Userop receipt: {userop_receipt:?}");

    assert!(userop_receipt.success, "userop should succeed");
    assert!(
        provider.is_spent(&pool, note.nullifier_hash()).await?,
        "note should be spent"
    );

    Ok(())
}

/// The fee must be driven down to what the operation actually costs, so that the rest of the note
/// reaches the recipient during validation and the operation's own calls can spend it.
#[tokio::test]
#[ignore = "run with `cargo test --release -- --ignored`"]
async fn test_tornadocash_paymaster_flashcall() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    let anvil = Anvil::new().try_spawn()?;
    let wallet = anvil.wallet().expect("anvil provides a dev wallet");
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(anvil.endpoint_url())
        .erased();
    let chain_id = provider.get_chain_id().await?;

    let entrypoint = deploy_entry_point(&provider).await?;
    deploy_simple_account(&provider).await?;

    let mut pool = deploy_pool(provider.clone(), None).await?;
    let paymaster_address = deploy_paymaster(
        provider.clone(),
        entrypoint,
        PLACEHOLDER_FACTORY,
        PLACEHOLDER_WETH,
    )
    .await?;
    let adapter_address =
        deploy_fee_adapter(provider.clone(), paymaster_address, pool.address).await?;
    pool.paymaster = Some(PaymasterInfo {
        address: paymaster_address,
        adapter: adapter_address,
    });

    // Deposit a note
    info!("Depositing into pool");
    let deposit = Deposit::new(&pool, rand::random());
    let note = deposit.note.clone();
    provider
        .send_transaction(deposit.call().into())
        .await?
        .watch()
        .await?;

    // Sync a Merkle tree against the provider
    let syncer = RpcSyncer::new(provider.clone());
    let mut tree = MerkleTree::new();
    let snapshot = syncer.sync(&pool, ..).await?;
    tree.splice_events(&snapshot)?;

    info!("Starting local alto bundler");
    let alto = AltoBuilder::new(
        anvil.endpoint(),
        entrypoint,
        ALTO_EXECUTOR_PK,
        ALTO_UTILITY_PK,
    )
    .prefund(&provider)
    .await?
    .spawn()
    .await?;

    // Spend 90% of the note within the same operation that withdraws it
    let spend = U256::from(pool.amount_wei) * U256::from(90) / U256::from(100);
    let owner = PrivateKeySigner::random();
    let smart_account = Simple7702SmartAccount::new(provider.clone(), owner.address(), chain_id);

    let builder = UserOperationBuilder::new_with_smart_account(&smart_account)
        .await?
        .with_call(&vec![Call {
            target: SINK,
            value: spend,
            ..Default::default()
        }]);

    let sink_before = provider.get_balance(SINK).await?;

    info!("Withdrawing {spend} wei and spending it in the same operation");
    let withdrawal = Withdrawal::new(&pool, note, owner.address());
    let withdrawal_merkle_proof = tree.note_proof(&withdrawal.note)?;
    let builder = builder
        .with_tornado_paymaster(
            withdrawal,
            &withdrawal_merkle_proof,
            &provider,
            &*alto,
            &mut rand::rng(),
        )
        .await?;

    let userop = builder.build().sign(&owner).await?;
    let userop_hash = alto.send_user_operation(&userop).await?;
    let userop_receipt = alto.wait_for_receipt(userop_hash).await?;
    info!("Userop receipt: {userop_receipt:?}");

    assert!(
        userop_receipt.success,
        "flashcall should succeed, but reverted: {:?}",
        userop_receipt.reason
    );

    let sink_after = provider.get_balance(SINK).await?;
    assert_eq!(
        sink_after - sink_before,
        spend,
        "sink should have received the flashcalled funds"
    );

    Ok(())
}
