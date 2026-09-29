use alloy::{primitives::Address, signers::local::PrivateKeySigner};
use kohaku_stealth::{Announcement, Scheme3Account, scheme3::try_match_announcement};
use serde::Deserialize;

#[derive(Deserialize)]
struct TypeScriptFixture {
    producer: String,
    keygen_master: String,
    keygen_index: u64,
    sender_master: String,
    sender_index: u64,
    meta_address: String,
    stealth_address: String,
    ephemeral_public_key: String,
    metadata: String,
}

#[test]
fn scans_and_spends_typescript_announcement() {
    let fixture: TypeScriptFixture =
        serde_json::from_str(include_str!("fixtures/typescript-announcement.json")).unwrap();
    let keygen_master = hex::decode(fixture.keygen_master).unwrap();
    let derived = Scheme3Account::from_keygen_master(&keygen_master).unwrap();

    assert_eq!(fixture.producer, "@kohaku-eth/pq-stealth-scheme3@0.1.1");
    assert_eq!(derived.keygen_index, fixture.keygen_index);
    assert_eq!(fixture.sender_master, "09".repeat(32));
    assert_eq!(fixture.sender_index, 0);
    assert_eq!(
        derived.account.meta_address().as_bytes(),
        hex::decode(fixture.meta_address).unwrap()
    );

    let stealth_address = fixture.stealth_address.parse::<Address>().unwrap();
    let announcement = Announcement::from_parts(
        stealth_address,
        hex::decode(fixture.ephemeral_public_key).unwrap(),
        hex::decode(fixture.metadata).unwrap(),
    )
    .unwrap();
    let scanner = derived.account.scanner().unwrap();
    let matched = try_match_announcement(&scanner, &announcement).unwrap();

    assert_eq!(matched.stealth_address(), stealth_address);
    let private_key = derived
        .account
        .derive_stealth_private_key(&matched)
        .unwrap();
    let signer = PrivateKeySigner::from_slice(private_key.expose_secret()).unwrap();
    assert_eq!(signer.address(), stealth_address);
}
