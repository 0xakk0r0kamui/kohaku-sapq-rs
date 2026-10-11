use alloy::{
    network::TransactionBuilder,
    node_bindings::Anvil,
    primitives::{Address, Bytes, U256, address},
    providers::{Provider, ProviderBuilder, ext::AnvilApi},
    rpc::types::TransactionRequest,
    signers::{SignerSync, local::PrivateKeySigner},
    sol,
    sol_types::{SolCall, SolStruct, eip712_domain},
};
use futures::{TryStreamExt, pin_mut, stream};
use kohaku_stealth::{
    AccountSeed, Deployment, FetchConfig, FetchError, FetchEvent, SCHEME_ID, ScanEvent,
    Scheme3Account, SchemeError,
    contracts::{
        IERC5564Announcer, IERC6538Registry, prepare_announcement, prepare_registration,
        prepare_registration_on_behalf, resolve_meta_address,
    },
    generate_stealth_address, generate_stealth_address_with_seed, get_announcements,
    get_announcements_for_user,
};
use pqsa_core::{SenderState, StealthScheme};
use pqsa_per_payment::SchemeId3;
use rand::{SeedableRng, rngs::StdRng};

#[path = "fixtures/mod.rs"]
mod fixtures;

const ANNOUNCER: Address = address!("55649E01B5Df198D18D95b5cc5051630cfD45564");
const REGISTRY: Address = address!("6538E6bf4B0eBd30A8Ea093027Ac2422ce5d6538");

sol! {
    struct Erc6538RegistryEntry {
        uint256 schemeId;
        bytes stealthMetaAddress;
        uint256 nonce;
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("kohaku_stealth=debug")),
        )
        .with_test_writer()
        .try_init();
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn receive_and_spend() -> anyhow::Result<()> {
    init_tracing();
    let anvil = Anvil::new().try_spawn()?;
    let url = anvil.endpoint_url();
    let funder = PrivateKeySigner::from(anvil.keys()[0].clone());
    let recipient = PrivateKeySigner::from(anvil.keys()[1].clone());
    let provider = ProviderBuilder::new()
        .wallet(funder.clone())
        .connect_http(url.clone())
        .erased();
    let recipient_provider = ProviderBuilder::new()
        .wallet(recipient.clone())
        .connect_http(url.clone());
    for (address, runtime) in [
        (ANNOUNCER, include_str!("contracts/ERC5564Announcer.hex")),
        (REGISTRY, include_str!("contracts/ERC6538Registry.hex")),
    ] {
        let code = runtime.trim().parse::<Bytes>()?;
        provider.anvil_set_code(address, code.clone()).await?;
        assert_eq!(provider.get_code_at(address).await?, code);
    }
    let deployment = Deployment {
        chain_id: provider.get_chain_id().await?,
        announcer: ANNOUNCER,
        registry: REGISTRY,
        start_block: 0,
    };
    let fixture = fixtures::Payment::load();
    let account = fixture.account();
    assert_eq!(fixture.scheme_id, SCHEME_ID);
    assert!(
        resolve_meta_address(&provider, &deployment, recipient.address())
            .await?
            .is_none()
    );

    let registration_receipt = recipient_provider
        .send_transaction(prepare_registration(&deployment, account.meta_address())?)
        .await?
        .get_receipt()
        .await?;
    assert!(registration_receipt.status());
    assert_eq!(registration_receipt.from, recipient.address());
    let registered = resolve_meta_address(&provider, &deployment, recipient.address())
        .await?
        .expect("meta-address was registered");
    assert_eq!(
        SchemeId3::meta_to_bytes(&registered),
        fixture.meta_address.as_ref()
    );

    let seed = SenderState::resume([0x5a; 32], 0)
        .draw_seed::<SchemeId3>()
        .map_err(SchemeError::from)?;
    let announcement = generate_stealth_address_with_seed(&registered, &seed)?;
    let published = fixture.announcement();
    assert_eq!(
        SchemeId3::announcement_to_bytes(&announcement),
        SchemeId3::announcement_to_bytes(&published)
    );
    let announcement_receipt = provider
        .send_transaction(prepare_announcement(&deployment, &announcement)?)
        .await?
        .get_receipt()
        .await?;
    assert!(announcement_receipt.status());
    assert_eq!(announcement_receipt.from, funder.address());
    let funding_receipt = provider
        .send_transaction(
            TransactionRequest::default()
                .with_chain_id(deployment.chain_id)
                .with_to(fixture.stealth_address)
                .with_value(U256::from(1_000_000_000_000_000_000u128)),
        )
        .await?
        .get_receipt()
        .await?;
    assert!(funding_receipt.status());
    assert_eq!(funding_receipt.from, funder.address());

    let mut rng = StdRng::from_seed([0x52; 32]);
    let other_seed = AccountSeed::generate(&mut rng);
    let other_account = Scheme3Account::from_seed(other_seed.as_ref())?;
    let other_announcement = generate_stealth_address(other_account.meta_address(), &mut rng)?;
    let foreign_receipt = provider
        .send_transaction(prepare_announcement(&deployment, &other_announcement)?)
        .await?
        .get_receipt()
        .await?;
    assert!(foreign_receipt.status());

    for (scheme_id, metadata) in [
        (SCHEME_ID + 1, fixture.metadata.to_vec()),
        (
            SCHEME_ID,
            fixture.metadata[..fixture.metadata.len() - 1].to_vec(),
        ),
    ] {
        let call = IERC5564Announcer::announceCall {
            schemeId: U256::from(scheme_id),
            stealthAddress: fixture.stealth_address,
            ephemeralPubKey: fixture.epk_field.clone(),
            metadata: metadata.into(),
        };
        let receipt = provider
            .send_transaction(
                TransactionRequest::default()
                    .with_to(ANNOUNCER)
                    .with_input(call.abi_encode()),
            )
            .await?
            .get_receipt()
            .await?;
        assert!(receipt.status());
    }

    let unfunded_announcement = generate_stealth_address(&registered, &mut rng)?;
    let unfunded_receipt = provider
        .send_transaction(prepare_announcement(&deployment, &unfunded_announcement)?)
        .await?
        .get_receipt()
        .await?;
    assert!(unfunded_receipt.status());
    assert_eq!(
        provider
            .get_balance(Address::from(unfunded_announcement.stealth_address))
            .await?,
        U256::ZERO
    );

    // Advance past the announcements before fetching finalized logs.
    provider.anvil_mine(Some(64), None).await?;
    let fetched = get_announcements(&provider, deployment, 0..=u64::MAX, FetchConfig::default())
        .try_collect::<Vec<_>>()
        .await?;
    assert_eq!(
        fetched
            .iter()
            .filter(|event| matches!(event, FetchEvent::Announcement(_)))
            .count(),
        3
    );
    assert!(matches!(
        fetched.last(),
        Some(FetchEvent::RangeFetched {
            rejected_logs: 1,
            ..
        })
    ));

    let scanner = account.scanner()?;
    let events = get_announcements_for_user(
        &scanner,
        stream::iter(fetched.into_iter().map(Ok::<_, FetchError>)),
    );
    pin_mut!(events);
    let mut matches = Vec::new();
    let mut completed = false;
    while let Some(event) = events.try_next().await? {
        assert!(!completed, "match followed range completion");
        match event {
            ScanEvent::Match(found) => matches.push(found),
            ScanEvent::RangeScanned { rejected_logs, .. } => {
                assert_eq!(rejected_logs, 1);
                completed = true;
            }
        }
    }
    assert!(completed);
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].stealth_address(), fixture.stealth_address);
    assert_eq!(
        matches[0].record().transaction_hash(),
        announcement_receipt.transaction_hash
    );
    assert_eq!(
        matches[1].stealth_address(),
        Address::from(unfunded_announcement.stealth_address)
    );
    assert_eq!(
        matches[1].record().transaction_hash(),
        unfunded_receipt.transaction_hash
    );

    let private_key = matches[0].spend_key(&account)?;
    assert_eq!(
        private_key.expose_secret().as_slice(),
        fixture.spend_key.as_ref()
    );
    let spender = PrivateKeySigner::from_slice(private_key.expose_secret())?;
    assert_eq!(spender.address(), fixture.stealth_address);

    let balance_before = provider.get_balance(funder.address()).await?;
    let spender_provider = ProviderBuilder::new()
        .wallet(spender)
        .connect_http(url)
        .erased();
    let spend_receipt = spender_provider
        .send_transaction(
            TransactionRequest::default()
                .with_to(funder.address())
                .with_value(U256::from(1000)),
        )
        .await?
        .get_receipt()
        .await?;
    assert!(spend_receipt.status());
    assert_eq!(
        provider.get_balance(funder.address()).await?,
        balance_before + U256::from(1000)
    );
    Ok(())
}

#[tokio::test]
async fn registration_on_behalf() -> anyhow::Result<()> {
    init_tracing();
    let anvil = Anvil::new().try_spawn()?;
    let relayer = PrivateKeySigner::from(anvil.keys()[0].clone());
    let registrant = PrivateKeySigner::from(anvil.keys()[1].clone());
    let provider = ProviderBuilder::new()
        .wallet(relayer.clone())
        .connect_http(anvil.endpoint_url())
        .erased();
    provider
        .anvil_set_code(
            REGISTRY,
            include_str!("contracts/ERC6538Registry.hex")
                .trim()
                .parse()?,
        )
        .await?;
    let registry = IERC6538Registry::new(REGISTRY, &provider);
    let deployment = Deployment {
        chain_id: provider.get_chain_id().await?,
        announcer: ANNOUNCER,
        registry: REGISTRY,
        start_block: 0,
    };
    let fixture = fixtures::Payment::load();
    let meta_address = SchemeId3::meta_from_bytes(&fixture.meta_address).unwrap();
    assert!(
        resolve_meta_address(&provider, &deployment, registrant.address())
            .await?
            .is_none()
    );
    let entry = Erc6538RegistryEntry {
        schemeId: U256::from(SCHEME_ID),
        stealthMetaAddress: fixture.meta_address.clone(),
        nonce: registry.nonceOf(registrant.address()).call().await?,
    };
    let domain = eip712_domain! {
        name: "ERC6538Registry",
        version: "1.0",
        chain_id: deployment.chain_id,
        verifying_contract: REGISTRY,
    };
    assert_eq!(
        domain.hash_struct(),
        registry.DOMAIN_SEPARATOR().call().await?
    );
    let signature = registrant.sign_hash_sync(&entry.eip712_signing_hash(&domain))?;
    let transaction = prepare_registration_on_behalf(
        &deployment,
        registrant.address(),
        &signature.as_bytes(),
        &meta_address,
    )?;
    let receipt = provider
        .send_transaction(transaction.clone())
        .await?
        .get_receipt()
        .await?;
    assert!(receipt.status());
    assert_eq!(receipt.from, relayer.address());
    let registered = resolve_meta_address(&provider, &deployment, registrant.address())
        .await?
        .expect("meta-address was registered on behalf");
    assert_eq!(
        SchemeId3::meta_to_bytes(&registered),
        fixture.meta_address.as_ref()
    );
    assert!(
        resolve_meta_address(&provider, &deployment, relayer.address())
            .await?
            .is_none()
    );
    assert_eq!(
        registry.nonceOf(registrant.address()).call().await?,
        entry.nonce + U256::from(1)
    );

    let replay = provider
        .send_transaction(transaction.with_gas_limit(200_000))
        .await?
        .get_receipt()
        .await?;
    assert!(!replay.status());
    assert!(replay.gas_used < 200_000);
    assert_eq!(
        registry.nonceOf(registrant.address()).call().await?,
        entry.nonce + U256::from(1)
    );
    Ok(())
}
