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
    Announcement, Deployment, SCHEME_ID, Scheme3Account, StealthProvider,
    scheme3::generate_stealth_address_with_seed,
};
use rand::{SeedableRng, rngs::StdRng};

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

fn account_seed() -> [u8; 128] {
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

#[test]
fn payment_builder_uses_fresh_rng_output() -> anyhow::Result<()> {
    subscriber();
    let deployment = Deployment {
        chain_id: 1,
        announcer: Address::repeat_byte(0x11),
        registry: Address::repeat_byte(0x22),
        start_block: 0,
    };
    let provider = ProviderBuilder::new()
        .connect_http("http://localhost:8545".parse()?)
        .erased();
    let client = StealthProvider::rpc(&Store::create(), provider, deployment);
    let account = Scheme3Account::from_seed(&account_seed())?;
    let mut rng = StdRng::from_seed([0x42; 32]);

    let first = client
        .payment(account.meta_address())
        .native(U256::from(1))
        .prepare(&mut rng)?;
    let second = client
        .payment(account.meta_address())
        .native(U256::from(1))
        .prepare(&mut rng)?;

    assert_ne!(first.stealth_address, second.stealth_address);
    assert_ne!(first.announcement, second.announcement);
    Ok(())
}

#[tokio::test]
async fn register_announce_sync_scan_and_spend() -> anyhow::Result<()> {
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
        chain_id: provider.get_chain_id().await?,
        announcer: *announcer.address(),
        registry: *registry.address(),
        start_block: 0,
    };
    let client = StealthProvider::rpc(&Store::create(), provider.clone(), deployment);
    let account = Scheme3Account::from_seed(&account_seed())?;

    provider
        .send_transaction(client.prepare_registration(account.meta_address()))
        .await?
        .get_receipt()
        .await?;
    let registered = client.resolve_meta_address(funder.address()).await?;
    assert_eq!(&registered, account.meta_address());

    let delegated = client.prepare_registration_on_behalf(
        funder.address(),
        &[0x77; 65],
        account.meta_address(),
    );
    assert!(!delegated.input.input().unwrap_or_default().is_empty());

    let mut rng = StdRng::from_seed([0x52; 32]);
    let payment = client
        .payment(account.meta_address())
        .native(U256::from(1_000_000_000_000_000_000u128))
        .prepare(&mut rng)?;
    let announcement_receipt = provider
        .send_transaction(payment.announcement_transaction.clone())
        .await?
        .get_receipt()
        .await?;
    assert!(announcement_receipt.status());
    let funding_receipt = provider
        .send_transaction(payment.funding_transaction.clone())
        .await?
        .get_receipt()
        .await?;
    assert!(funding_receipt.status());

    announce_raw(&announcer, SCHEME_ID, 0xab, 1089).await?;
    announce_raw(&announcer, SCHEME_ID + 1, 0xcd, 1089).await?;
    announce_raw(&announcer, SCHEME_ID, 0x11, 2_000).await?;

    let report = client.sync().await?;
    assert_eq!(report.stored, 2, "rejected {}", report.rejected_logs);
    assert!(report.rejected_logs >= 1);
    assert_eq!(client.announcements().await?.len(), 2);
    let repeated = client.sync().await?;
    assert_eq!(repeated.stored, 2);

    let explicit = generate_stealth_address_with_seed(account.meta_address(), &[0x44; 64])?;
    let announce_only = client.prepare_announcement(&explicit.announcement);
    assert!(!announce_only.input.input().unwrap_or_default().is_empty());
    assert!(
        Announcement::from_parts(
            explicit.stealth_address,
            explicit.announcement.ephemeral_public_key(),
            vec![0u8; 2_000],
        )
        .is_err()
    );

    let scanner = account.scanner()?;
    let matches = client.matches(&scanner).await?;
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].stealth_address(), payment.stealth_address);
    assert_eq!(
        matches[0].record().transaction_hash(),
        announcement_receipt.transaction_hash
    );

    let private_key = matches[0].derive_stealth_private_key(&account)?;
    let spender = PrivateKeySigner::from_slice(private_key.expose_secret())?;
    assert_eq!(spender.address(), payment.stealth_address);

    let back = ProviderBuilder::new()
        .wallet(spender)
        .connect_http(url)
        .erased();
    let receipt = back
        .send_transaction(
            alloy::rpc::types::TransactionRequest::default()
                .with_to(funder.address())
                .with_value(U256::from(1000)),
        )
        .await?
        .get_receipt()
        .await?;
    assert!(receipt.status());
    Ok(())
}
