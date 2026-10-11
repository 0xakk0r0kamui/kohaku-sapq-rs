use alloy::{primitives::Address, signers::local::PrivateKeySigner, sol_types::SolCall};
use kohaku_stealth::{
    Deployment, EngineError, Scheme3Account, SchemeError, compute_stealth_spend_key,
    contracts::{IERC5564Announcer, IERC6538Registry, prepare_announcement, prepare_registration},
    scan_announcement,
    scheme3::bind_scanner,
};
use pqsa_core::StealthScheme;
use pqsa_per_payment::SchemeId3;

#[path = "fixtures/mod.rs"]
mod fixtures;

#[test]
fn scans_and_spends_upstream_announcement() {
    let fixture = fixtures::Payment::load();
    let account = fixture.account();
    assert_eq!(fixture.scheme_id, kohaku_stealth::SCHEME_ID);
    let announcement = fixture.announcement();
    let scanner = account.scanner().unwrap();
    let matched = scan_announcement(&scanner, &announcement).unwrap();

    assert_eq!(
        Address::from(matched.stealth_address),
        fixture.stealth_address
    );
    let private_key = account.spend_key(&matched).unwrap();
    assert_eq!(
        private_key.expose_secret().as_slice(),
        fixture.spend_key.as_ref()
    );
    let signer = PrivateKeySigner::from_slice(private_key.expose_secret()).unwrap();
    assert_eq!(signer.address(), fixture.stealth_address);
}

#[test]
fn engine_types() {
    let fixture = fixtures::Payment::load();
    let account = fixture.account();
    let meta: pqsa_per_payment::MetaAddress =
        SchemeId3::meta_from_bytes(&fixture.meta_address).unwrap();
    let scanner: pqsa_per_payment::Scanner = bind_scanner(account.tracking_key(), &meta).unwrap();

    let generated = fixture.announcement();
    let matched = scan_announcement(&scanner, &generated).unwrap();
    assert_eq!(
        Address::from(matched.stealth_address),
        Address::from(generated.stealth_address)
    );

    let deployment = Deployment {
        chain_id: 1,
        announcer: Address::repeat_byte(0x11),
        registry: Address::repeat_byte(0x22),
        start_block: 0,
    };
    let registration = prepare_registration(&deployment, &meta).unwrap();
    let registration_call =
        IERC6538Registry::registerKeysCall::abi_decode(registration.input.input().unwrap())
            .unwrap();
    assert_eq!(
        registration_call.stealthMetaAddress.as_ref(),
        SchemeId3::meta_to_bytes(&meta)
    );
    let announcement = prepare_announcement(&deployment, &generated).unwrap();
    let announcement_call =
        IERC5564Announcer::announceCall::abi_decode(announcement.input.input().unwrap()).unwrap();
    assert_eq!(
        announcement_call.stealthAddress,
        Address::from(generated.stealth_address)
    );
    assert_eq!(
        announcement_call.ephemeralPubKey.as_ref(),
        generated.epk.as_ref().unwrap().as_bytes()
    );
    assert_eq!(announcement_call.metadata[0], generated.view_tag[0]);
    assert_eq!(&announcement_call.metadata[1..], generated.ct);

    let other = Scheme3Account::from_keygen_master(&[8; 32])
        .unwrap()
        .account;
    assert_eq!(
        bind_scanner(account.tracking_key(), other.meta_address()).unwrap_err(),
        SchemeError::Engine(EngineError::TrackingKeyMismatch)
    );
}

#[test]
fn spend_key_interop() {
    let fixture = fixtures::Payment::load();
    let account = fixture.account();
    let scanner = SchemeId3::bind(account.tracking_key(), account.meta_address()).unwrap();
    let announcement = fixture.announcement();
    let sdk_match = scan_announcement(&scanner, &announcement).unwrap();
    let engine_match = SchemeId3::scan(&scanner, &announcement).unwrap();

    let sdk_key = compute_stealth_spend_key(account.master_key(), &engine_match).unwrap();
    let engine_key =
        zeroize::Zeroizing::new(SchemeId3::spend_key(account.master_key(), &sdk_match).unwrap());
    assert_eq!(sdk_key.expose_secret(), &*engine_key);
    let signer = PrivateKeySigner::from_slice(sdk_key.expose_secret()).unwrap();
    assert_eq!(
        signer.address(),
        Address::from(announcement.stealth_address)
    );
}
