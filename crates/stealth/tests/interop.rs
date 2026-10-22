use alloy::{
    primitives::{Address, B256, Bytes, TxKind, U256, hex},
    providers::ProviderBuilder,
    rpc::{
        client::RpcClient,
        json_rpc::ErrorPayload,
        types::{Block, Log},
    },
    signers::local::PrivateKeySigner,
    sol_types::{SolCall, SolEvent},
    transports::{
        layers::RetryBackoffLayer,
        mock::{Asserter, MockTransport},
    },
};
use futures::{TryStreamExt, pin_mut};
use kohaku_stealth::{
    Account, AccountSeed, Deployment, EngineError, ExposeSecret, FetchError, FetchEvent, ScanEvent,
    SchemeId3, StealthScheme, StreamedEventConfig,
    contracts::{prepare_announcement, register_keys},
    generate_stealth_address, get_announcements, get_announcements_for_user,
};

#[path = "../src/abis.rs"]
mod abis;
use abis::{IERC5564Announcer, IERC6538Registry};

#[path = "fixtures/mod.rs"]
mod fixtures;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn scans_and_spends_upstream_announcement() {
    let fixture = fixtures::Payment::load();
    let account = fixture.account();
    assert_eq!(fixture.scheme_id, SchemeId3::SCHEME_ID);
    let meta: &pqsa_per_payment::MetaAddress = account.meta_address();
    let master: &pqsa_per_payment::Master = account.master_key();
    let tracking: &pqsa_per_payment::Tracking = account.tracking_key();
    let announcement: pqsa_per_payment::Announcement = fixture.announcement();
    let scanner: pqsa_per_payment::Scanner = SchemeId3::bind(tracking, meta).unwrap();
    let matched: pqsa_per_payment::Match = SchemeId3::scan(&scanner, &announcement).unwrap();

    assert_eq!(
        Address::from(matched.stealth_address),
        fixture.stealth_address
    );
    let private_key: pqsa_per_payment::SpendKey = SchemeId3::spend_key(master, &matched).unwrap();
    assert_eq!(
        private_key.expose_secret().as_slice(),
        fixture.spend_key.as_ref()
    );
    let signer = PrivateKeySigner::from_slice(private_key.expose_secret()).unwrap();
    assert_eq!(signer.address(), fixture.stealth_address);

    let other = Account::from_keygen_master(&[8; 32]).unwrap().account;
    assert_eq!(
        SchemeId3::bind(tracking, other.meta_address()).unwrap_err(),
        EngineError::TrackingKeyMismatch
    );
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn prepares_transactions_from_upstream_types() {
    let fixture = fixtures::Payment::load();
    let meta: pqsa_per_payment::MetaAddress =
        SchemeId3::meta_from_bytes(&fixture.meta_address).unwrap();
    let generated = fixture.announcement();

    let deployment = Deployment {
        chain_id: 1,
        announcer: Address::repeat_byte(0x11),
        registry: Address::repeat_byte(0x22),
        start_block: 0,
    };
    let registration = register_keys(&deployment, &meta);
    assert_eq!(registration.to, Some(TxKind::Call(deployment.registry)));
    assert_eq!(registration.chain_id, Some(deployment.chain_id));
    let registration_call =
        IERC6538Registry::registerKeysCall::abi_decode(registration.input.input().unwrap())
            .unwrap();
    assert_eq!(
        registration_call.stealthMetaAddress.as_ref(),
        fixture.meta_address.as_ref()
    );
    assert_eq!(registration_call.schemeId, U256::from(fixture.scheme_id));
    let announcement = prepare_announcement(&deployment, &generated);
    assert_eq!(announcement.to, Some(TxKind::Call(deployment.announcer)));
    assert_eq!(announcement.chain_id, Some(deployment.chain_id));
    let announcement_call =
        IERC5564Announcer::announceCall::abi_decode(announcement.input.input().unwrap()).unwrap();
    assert_eq!(announcement_call.schemeId, U256::from(fixture.scheme_id));
    assert_eq!(
        announcement_call.stealthAddress,
        Address::from(generated.stealth_address)
    );
    assert_eq!(
        announcement_call.ephemeralPubKey.as_ref(),
        fixture.ephemeral_pub_key.as_ref()
    );
    assert_eq!(announcement_call.metadata, fixture.metadata);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn random_announcements_match_the_account() {
    let mut rng = rand::rng();
    let seed = AccountSeed::generate(&mut rng);
    let account = Account::from_seed(seed.expose_secret()).unwrap();
    let scanner: pqsa_per_payment::Scanner = account.scanner().unwrap();
    let first = generate_stealth_address(account.meta_address(), &mut rng).unwrap();
    let second = generate_stealth_address(account.meta_address(), &mut rng).unwrap();
    assert_ne!(first.stealth_address, second.stealth_address);

    for announcement in [&first, &second] {
        let matched = SchemeId3::scan(&scanner, announcement).unwrap();
        let key: pqsa_per_payment::SpendKey = account.spend_key(&matched).unwrap();
        let signer = PrivateKeySigner::from_slice(key.expose_secret()).unwrap();
        assert_eq!(
            signer.address(),
            Address::from(announcement.stealth_address)
        );
    }
    assert_ne!(PrivateKeySigner::random().address(), Address::ZERO);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
async fn fetches_and_scans_upstream_announcement() {
    let fixture = fixtures::Payment::load();
    let account = fixture.account();
    let scanner = account.scanner().unwrap();
    let deployment = Deployment {
        chain_id: 1,
        announcer: Address::repeat_byte(0x11),
        registry: Address::repeat_byte(0x22),
        start_block: 0,
    };
    let block_hash = B256::repeat_byte(0x44);
    let transaction_hash = B256::repeat_byte(0x55);
    let mut token_metadata = fixture.metadata.to_vec();
    // Token block from upstream's V3-08a.
    token_metadata.extend_from_slice(&hex!(
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee0000000000000000000000000000000000000000000000000de0b6b3a7640000"
    ));
    let mut other_token_metadata = token_metadata.clone();
    *other_token_metadata.last_mut().unwrap() ^= 1;
    let announcements = [
        (fixture.stealth_address, fixture.metadata.clone()),
        (fixture.stealth_address, token_metadata.into()),
        (fixture.stealth_address, other_token_metadata.into()),
        (Address::ZERO, fixture.metadata.clone()),
        (fixture.stealth_address, Bytes::new()),
    ];
    let logs: Vec<_> = announcements
        .iter()
        .enumerate()
        .map(|(index, (address, metadata))| {
            let event = IERC5564Announcer::Announcement {
                schemeId: U256::from(fixture.scheme_id),
                stealthAddress: *address,
                caller: Address::repeat_byte(0xbb),
                ephemeralPubKey: fixture.ephemeral_pub_key.clone(),
                metadata: metadata.clone(),
            };
            Log {
                inner: alloy::primitives::Log {
                    address: deployment.announcer,
                    data: event.encode_log_data(),
                },
                block_number: Some(1),
                block_hash: Some(block_hash),
                transaction_hash: Some(transaction_hash),
                log_index: Some(index as u64),
                ..Default::default()
            }
        })
        .collect();
    let asserter = Asserter::new();
    asserter.push_success(&"0x1");
    let mut head = Block::<alloy::rpc::types::Transaction>::default();
    head.header.number = 1;
    asserter.push_success(&head);
    asserter.push_success(&logs);
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    let source = get_announcements(
        &provider,
        deployment,
        1..=1,
        StreamedEventConfig {
            max_retries: 0,
            ..Default::default()
        },
    )
    .inspect_ok(|event| {
        if let FetchEvent::Announcement(record) = event {
            let index = usize::try_from(record.log_index).unwrap();
            assert_eq!(&record.metadata, &announcements[index].1);
        }
    });
    let events = get_announcements_for_user(&scanner, source);
    pin_mut!(events);
    for (index, (_, metadata)) in announcements[..3].iter().enumerate() {
        let event = events.try_next().await.unwrap();
        let retained = event.clone();
        drop(event);
        let Some(ScanEvent::Match(matched)) = retained else {
            panic!("expected the fixture's announcement at index {index}");
        };
        assert_eq!(matched.stealth_address(), fixture.stealth_address);
        assert_eq!(matched.record.block_hash, block_hash);
        assert_eq!(matched.record.transaction_hash, transaction_hash);
        assert_eq!(matched.record.log_index, index as u64);
        assert_eq!(&matched.record.metadata, metadata);
        let key: pqsa_per_payment::SpendKey = account.spend_key(&matched.match_result).unwrap();
        assert_eq!(key.expose_secret().as_slice(), fixture.spend_key.as_ref());
    }
    assert!(matches!(events.try_next().await.unwrap(),
        Some(ScanEvent::RangeScanned { range, rejected_logs: 1 }) if range == (1..=1)));
    assert!(events.try_next().await.unwrap().is_none());
    assert!(asserter.read_q().is_empty());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
async fn retries_rate_limited_fetch() {
    let deployment = Deployment {
        chain_id: 1,
        announcer: Address::repeat_byte(0x11),
        registry: Address::repeat_byte(0x22),
        start_block: 0,
    };
    let asserter = Asserter::new();
    asserter.push_success(&"0x1");
    let mut head = Block::<alloy::rpc::types::Transaction>::default();
    head.header.number = 1;
    asserter.push_success(&head);
    asserter.push_failure(ErrorPayload {
        code: 429,
        message: "rate limited".into(),
        data: None,
    });
    asserter.push_success(&Vec::<Log>::new());
    let client = RpcClient::builder()
        .layer(RetryBackoffLayer::new(1, 1, 1_000))
        .transport(MockTransport::new(asserter.clone()), true);
    let provider = ProviderBuilder::new().connect_client(client);
    let events = get_announcements(
        &provider,
        deployment,
        1..=1,
        StreamedEventConfig {
            max_retries: 0,
            ..Default::default()
        },
    );
    pin_mut!(events);
    assert!(matches!(events.try_next().await.unwrap(),
        Some(FetchEvent::RangeFetched { range, rejected_logs: 0 }) if range == (1..=1)));
    assert!(events.try_next().await.unwrap().is_none());
    assert!(asserter.read_q().is_empty());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
async fn rate_limit_exhausts_retries() {
    let deployment = Deployment {
        chain_id: 1,
        announcer: Address::repeat_byte(0x11),
        registry: Address::repeat_byte(0x22),
        start_block: 0,
    };
    for max_retries in [0, 1] {
        let asserter = Asserter::new();
        asserter.push_success(&"0x1");
        let mut head = Block::<alloy::rpc::types::Transaction>::default();
        head.header.number = 1;
        asserter.push_success(&head);
        for _ in 0..=max_retries {
            asserter.push_failure(ErrorPayload {
                code: 429,
                message: "rate limited".into(),
                data: None,
            });
        }
        let client = RpcClient::builder()
            .layer(RetryBackoffLayer::new(max_retries, 1, 1_000))
            .transport(MockTransport::new(asserter.clone()), true);
        let provider = ProviderBuilder::new().connect_client(client);
        let events = get_announcements(
            &provider,
            deployment,
            1..=1,
            StreamedEventConfig {
                max_retries: 0,
                ..Default::default()
            },
        );
        pin_mut!(events);
        let FetchError::Rpc(error) = events.try_next().await.unwrap_err() else {
            panic!("expected a transport error");
        };
        let message = error.to_string();
        assert!(message.contains("Max retries exceeded"), "{message}");
        assert!(message.contains("429"), "{message}");
        assert!(events.try_next().await.unwrap().is_none());
        assert!(asserter.read_q().is_empty());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn addresses_are_not_logged() {
    use kohaku_stealth::{StealthProviderExt, contracts::register_keys_on_behalf};
    use std::sync::Mutex;
    use tracing::instrument::WithSubscriber;

    // Keep callsite interest dynamic while tests run in parallel.
    let _inactive = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    let capture = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_env_filter("kohaku_stealth=trace")
            .with_ansi(false)
            .without_time()
            .with_writer(Mutex::new(Vec::<u8>::new()))
            .finish(),
    );
    let fixture = fixtures::Payment::load();
    let deployment = Deployment {
        chain_id: 1,
        announcer: Address::repeat_byte(0x11),
        registry: Address::repeat_byte(0x22),
        start_block: 0,
    };
    let registrant = Address::repeat_byte(0x77);
    let asserter = Asserter::new();
    asserter.push_success(&"0x1");
    asserter.push_success(&Bytes::from(
        IERC6538Registry::stealthMetaAddressOfCall::abi_encode_returns(&fixture.meta_address),
    ));
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    let address = async {
        let meta = provider
            .resolve_meta_address(&deployment, registrant)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            SchemeId3::meta_to_bytes(&meta),
            fixture.meta_address.as_ref()
        );
        let generated = generate_stealth_address(&meta, &mut rand::rng()).unwrap();
        let _ = register_keys(&deployment, &meta);
        let _ = register_keys_on_behalf(&deployment, registrant, &[0x88; 65], &meta);
        let _ = prepare_announcement(&deployment, &generated);
        Address::from(generated.stealth_address)
    }
    .with_subscriber(capture.clone())
    .await;
    assert!(asserter.read_q().is_empty());
    let output = capture
        .downcast_ref::<Mutex<Vec<u8>>>()
        .unwrap()
        .lock()
        .unwrap();
    let logged = std::str::from_utf8(&output).unwrap();
    for event in [
        "resolved stealth meta-address",
        "generated stealth address",
        "prepared stealth-key registration",
        "prepared delegated stealth-key registration",
        "prepared stealth announcement",
    ] {
        assert!(logged.contains(event), "missing captured event: {event}");
    }
    let logged = logged.to_ascii_lowercase();
    for address in [registrant, address] {
        assert!(!logged.contains(&hex::encode(address)));
    }
}
