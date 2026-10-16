use alloy::{
    primitives::{Address, B256, Bytes, TxKind, U256, hex},
    providers::ProviderBuilder,
    rpc::types::{Block, Log},
    signers::local::PrivateKeySigner,
    sol_types::{SolCall, SolEvent},
    transports::mock::Asserter,
};
use futures::{TryStreamExt, pin_mut};
use kohaku_stealth::{
    AccountSeed, Deployment, EngineError, ExposeSecret, FetchConfig, ScanEvent, Scheme3Account,
    SchemeError, compute_stealth_spend_key,
    contracts::{IERC5564Announcer, IERC6538Registry, prepare_announcement, prepare_registration},
    generate_stealth_address, get_announcements, get_announcements_for_user, scan_announcement,
    scheme3::bind_scanner,
};
use pqsa_core::StealthScheme;
use pqsa_per_payment::SchemeId3;

#[path = "fixtures/mod.rs"]
mod fixtures;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn scans_and_spends_upstream_announcement() {
    let fixture = fixtures::Payment::load();
    let account = fixture.account();
    assert_eq!(fixture.scheme_id, kohaku_stealth::SCHEME_ID);
    let meta: &pqsa_per_payment::MetaAddress = account.meta_address();
    let master: &pqsa_per_payment::Master = account.master_key();
    let tracking: &pqsa_per_payment::Tracking = account.tracking_key();
    let announcement: pqsa_per_payment::Announcement = fixture.announcement();
    let scanner: pqsa_per_payment::Scanner = bind_scanner(tracking, meta).unwrap();
    let matched: pqsa_per_payment::Match = scan_announcement(&scanner, &announcement).unwrap();

    assert_eq!(
        Address::from(matched.stealth_address),
        fixture.stealth_address
    );
    let private_key: pqsa_per_payment::SpendKey =
        compute_stealth_spend_key(master, &matched).unwrap();
    assert_eq!(
        private_key.expose_secret().as_slice(),
        fixture.spend_key.as_ref()
    );
    let signer = PrivateKeySigner::from_slice(private_key.expose_secret()).unwrap();
    assert_eq!(signer.address(), fixture.stealth_address);

    let other = Scheme3Account::from_keygen_master(&[8; 32])
        .unwrap()
        .account;
    assert_eq!(
        bind_scanner(tracking, other.meta_address()).unwrap_err(),
        SchemeError::Engine(EngineError::TrackingKeyMismatch)
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
    let registration = prepare_registration(&deployment, &meta);
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
    let account = Scheme3Account::from_seed(seed.expose_secret()).unwrap();
    let scanner: pqsa_per_payment::Scanner = account.scanner().unwrap();
    let first = generate_stealth_address(account.meta_address(), &mut rng).unwrap();
    let second = generate_stealth_address(account.meta_address(), &mut rng).unwrap();
    assert_ne!(first.stealth_address, second.stealth_address);

    for announcement in [&first, &second] {
        let matched = scan_announcement(&scanner, announcement).unwrap();
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
    let logs: Vec<_> = [
        (fixture.stealth_address, fixture.metadata.clone()),
        (fixture.stealth_address, token_metadata.into()),
        (Address::ZERO, fixture.metadata.clone()),
        (fixture.stealth_address, Bytes::new()),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (address, metadata))| {
        let event = IERC5564Announcer::Announcement {
            schemeId: U256::from(fixture.scheme_id),
            stealthAddress: address,
            caller: Address::repeat_byte(0xbb),
            ephemeralPubKey: fixture.ephemeral_pub_key.clone(),
            metadata,
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
    let source = get_announcements(&provider, deployment, 1..=1, FetchConfig::default());
    let events = get_announcements_for_user(&scanner, source);
    pin_mut!(events);
    for index in 0..2 {
        let Some(ScanEvent::Match(matched)) = events.try_next().await.unwrap() else {
            panic!("expected the fixture's announcement at index {index}");
        };
        assert_eq!(matched.stealth_address(), fixture.stealth_address);
        assert_eq!(matched.record().block_hash(), block_hash);
        assert_eq!(matched.record().transaction_hash(), transaction_hash);
        assert_eq!(matched.record().log_index(), index);
        let key: pqsa_per_payment::SpendKey = matched.spend_key(&account).unwrap();
        assert_eq!(key.expose_secret().as_slice(), fixture.spend_key.as_ref());
    }
    assert!(matches!(events.try_next().await.unwrap(),
        Some(ScanEvent::RangeScanned { range, rejected_logs: 1 }) if range == (1..=1)));
    assert!(events.try_next().await.unwrap().is_none());
    assert!(asserter.read_q().is_empty());
}
