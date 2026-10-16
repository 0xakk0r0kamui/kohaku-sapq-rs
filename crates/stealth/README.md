# kohaku-stealth

Rust SDK for the hybrid stealth address scheme proposed in
[ERC-8441](https://github.com/ethereum/ERCs/pull/2059).
Announcements use [ERC-5564](https://eips.ethereum.org/EIPS/eip-5564).

## Announcements

Get the recipient's meta-address directly or look it up with
`contracts::resolve_meta_address`. An empty registry entry returns `Ok(None)`.

```rust,no_run
use alloy::{primitives::Address, rpc::types::TransactionRequest};
use kohaku_stealth::{
    Deployment, StealthMetaAddress, contracts::prepare_announcement, generate_stealth_address,
};

fn example(
    deployment: &Deployment,
    recipient: &StealthMetaAddress,
) -> Result<(Address, TransactionRequest), kohaku_stealth::SchemeError> {
    let mut rng = rand::rng();
    let generated = generate_stealth_address(recipient, &mut rng)?;
    let address = Address::from(generated.stealth_address);
    let transaction = prepare_announcement(deployment, &generated);
    Ok((address, transaction))
}
```

Submit the unsigned `transaction` with Alloy and confirm success before funding
`address`. Retry failed funding to the same address. Save each transaction hash
before waiting for a receipt; after a timeout, check its status before resending.

## Scanning

Create a scanner with `Scheme3Account::scanner`. For watch-only scanning, use
`scheme3::bind_scanner` with the tracking key and meta-address.

```rust,no_run
use alloy::providers::Provider;
use futures::{TryStreamExt, pin_mut};
use kohaku_stealth::{
    Deployment, FetchConfig, FetchError, ScanEvent, Scanner, get_announcements,
    get_announcements_for_user,
};

async fn receive(
    provider: &impl Provider,
    deployment: Deployment,
    scanner: &Scanner,
    from_block: u64,
) -> Result<(), FetchError> {
    let announcements = get_announcements(
        provider, deployment, from_block..=u64::MAX, FetchConfig::default(),
    );
    let events = get_announcements_for_user(scanner, announcements);
    pin_mut!(events);

    while let Some(event) = events.try_next().await? {
        match event {
            ScanEvent::Match(matched) => {
                println!("{} in {}", matched.stealth_address(), matched.record().transaction_hash());
            }
            ScanEvent::RangeScanned { range, rejected_logs } => {
                println!("scanned {range:?}; rejected {rejected_logs} malformed payloads");
            }
        }
    }
    Ok(())
}
```

Matches identify announcements for the scanner; check funding separately.
The stream scans through the finalized head read on the first poll. Start at the
announcer's deployment block unless you know when the recipient's keys were first
used. Payments can precede registration.

Save matches before checkpointing a range at `RangeScanned`. On error or
cancellation, replay the unfinished range and deduplicate by chain, announcer,
block hash, and log index. See `FetchConfig` for batch size and concurrency
settings.

## Keys

Store `AccountSeed` in a wallet keystore. Restore the account with
`Scheme3Account::from_seed`, then derive a spend key from a matched address.
With Alloy's `signer-local` feature:

```rust,no_run
use alloy::signers::local::PrivateKeySigner;
use kohaku_stealth::{ExposeSecret, MatchedStealthAddress, Scheme3Account};

fn example(
    account: &Scheme3Account,
    matched: &MatchedStealthAddress,
) -> Result<PrivateKeySigner, Box<dyn std::error::Error>> {
    let key = matched.spend_key(account)?;
    let signer = PrivateKeySigner::from_slice(key.expose_secret())?;
    Ok(signer)
}
```

`AccountSeed` and `SpendKey` clear their buffers on drop. A signer may keep its
own copy of the key.

`generate_stealth_address_with_seed` takes a 64-byte seed. Reusing a seed repeats
the ephemeral key and, for the same recipient, the stealth address. Use
`generate_stealth_address` with an RNG for new payments.

## Tests

See [tests/README.md][tests] for native and browser commands, fixture sources,
and update instructions. Set `RUST_LOG=kohaku_stealth=debug` for SDK logs.

[tests]: https://github.com/0xakk0r0kamui/kohaku-sapq-rs/blob/master/crates/stealth/tests/README.md
