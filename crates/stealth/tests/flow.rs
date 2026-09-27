use alloy::{
    network::TransactionBuilder,
    node_bindings::Anvil,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use kohaku_kv_store::Store;
use kohaku_stealth::{
    Asset, Deployment, SCHEME_ID, Scheme3Client, Transfer, announce_with_seed, keygen, spend_key,
};

sol!(
    #[sol(rpc)]
    Announcer,
    "tests/contracts/Announcer.json"
);

sol!(
    #[sol(rpc)]
    Registry,
    "tests/contracts/Registry.json"
);

fn keygen_seed() -> [u8; 128] {
    let mut seed = [0u8; 128];
    seed[..32].fill(0x11);
    seed[32..64].fill(0x22);
    seed[64..].fill(0x33);
    seed
}

async fn announce_raw(
    announcer: &Announcer::AnnouncerInstance<alloy::providers::DynProvider>,
    scheme_id: u64,
    address_byte: u8,
    metadata_len: usize,
) -> anyhow::Result<()> {
    let receipt = announcer
        .announce(
            U256::from(scheme_id),
            Address::repeat_byte(address_byte),
            vec![0x02; 33].into(),
            vec![0u8; metadata_len].into(),
        )
        .send()
        .await?
        .get_receipt()
        .await?;
    anyhow::ensure!(receipt.status());
    Ok(())
}

fn subscriber() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("kohaku_stealth=debug")),
        )
        .with_test_writer()
        .try_init();
}

#[tokio::test]
async fn sender_index_survives_a_new_client() -> anyhow::Result<()> {
    subscriber();
    let anvil = Anvil::new().try_spawn()?;
    let provider = ProviderBuilder::new()
        .connect_http(anvil.endpoint_url())
        .erased();
    let deployment = Deployment {
        announcer: Address::ZERO,
        registry: Address::ZERO,
        start_block: 0,
        scheme_id: SCHEME_ID,
    };
    let store = Store::create();
    let recipient = keygen(&keygen_seed())?;
    let master = [0x42; 32];
    let transfer = Transfer {
        asset: Asset::Native,
        amount: U256::from(1),
    };

    let bad = Deployment {
        scheme_id: SCHEME_ID + 1,
        ..deployment
    };
    assert!(Scheme3Client::new(&store, provider.clone(), bad).is_err());

    let first = Scheme3Client::new(&store, provider.clone(), deployment)?;
    let payment = first.prepare_payment(&recipient.meta_address, master, transfer).await?;
    assert_ne!(payment.stealth_address, Address::ZERO);

    let second = Scheme3Client::new(&store, provider, deployment)?;
    let again = second
        .prepare_payment(&recipient.meta_address, master, transfer)
        .await?;
    assert_ne!(payment.stealth_address, again.stealth_address);
    Ok(())
}

#[tokio::test]
async fn announce_sync_scan_and_spend() -> anyhow::Result<()> {
    subscriber();
    let anvil = Anvil::new().try_spawn()?;
    let url = anvil.endpoint_url();
    let funder_key = anvil.keys()[0].clone();
    let funder = PrivateKeySigner::from(funder_key);
    let provider = ProviderBuilder::new()
        .wallet(funder.clone())
        .connect_http(url.clone())
        .erased();

    let announcer = Announcer::deploy(provider.clone()).await?;
    let registry = Registry::deploy(provider.clone()).await?;
    let deployment = Deployment {
        announcer: *announcer.address(),
        registry: *registry.address(),
        start_block: 0,
        scheme_id: SCHEME_ID,
    };
    let store = Store::create();
    let client = Scheme3Client::new(&store, provider.clone(), deployment)?;

    let recipient = keygen(&keygen_seed())?;
    let register_tx = client.register_keys_tx(&recipient.meta_address);
    provider
        .send_transaction(register_tx)
        .await?
        .get_receipt()
        .await?;
    let registered = client.meta_address_of(funder.address()).await?;
    assert_eq!(registered, recipient.meta_address);

    let payment = client
        .prepare_payment(
            &recipient.meta_address,
            [0x52; 32],
            Transfer {
                asset: Asset::Native,
                amount: U256::from(1_000_000_000_000_000_000u128),
            },
        )
        .await?;
    provider
        .send_transaction(payment.announce.clone())
        .await?
        .get_receipt()
        .await?;
    provider
        .send_transaction(payment.fund.clone())
        .await?
        .get_receipt()
        .await?;

    announce_raw(&announcer, 3, 0xab, 1089).await?;
    announce_raw(&announcer, 4, 0xcd, 1089).await?;
    announce_raw(&announcer, SCHEME_ID, 0x11, 2_000).await?;

    let report = client.sync().await?;
    assert_eq!(report.stored, 2, "undecoded {}", report.undecoded);
    assert!(report.undecoded >= 1);
    assert_eq!(client.announcements().await?.len(), 2);

    let explicit = announce_with_seed(&recipient.meta_address, &[0x44; 64])?;
    let announce_only = client.announce_tx(&explicit)?;
    assert!(!announce_only.input.input().unwrap_or_default().is_empty());
    let mut malformed = explicit.clone();
    malformed.metadata.push(0);
    assert!(client.announce_tx(&malformed).is_err());

    let found = client
        .scan(&recipient.tracking, &recipient.meta_address)
        .await?;
    assert_eq!(found.len(), 1);
    assert_eq!(
        Address::from(found[0].stealth_address),
        payment.stealth_address
    );

    let scalar = spend_key(&recipient.master, &found[0])?;
    let spender = PrivateKeySigner::from_slice(&scalar)?;
    assert_eq!(spender.address(), payment.stealth_address);

    let back = ProviderBuilder::new()
        .wallet(spender)
        .connect_http(url)
        .erased();
    let tx = back
        .send_transaction(
            alloy::rpc::types::TransactionRequest::default()
                .with_to(funder.address())
                .with_value(U256::from(1000)),
        )
        .await?
        .get_receipt()
        .await?;
    assert!(tx.status());
    Ok(())
}
