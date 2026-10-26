# kohaku-stealth

Rust SDK for the hybrid stealth address scheme proposed in
[ERC-8441](https://github.com/ethereum/ERCs/pull/2059).
Announcements use [ERC-5564](https://eips.ethereum.org/EIPS/eip-5564).

## Announcements

Decode the recipient's meta-address:

```rust
use kohaku_stealth::{MetaAddress, SchemeId3, StealthScheme};

fn parse_recipient(bytes: &[u8]) -> Option<MetaAddress> {
    SchemeId3::meta_from_bytes(bytes)
}
```

To look it up in ERC-6538, import `StealthProviderExt` and call
`provider.resolve_meta_address(&deployment, registrant).await`. This returns
`Ok(None)` if no meta-address is registered.

Generate a stealth address and prepare its announcement transaction:

```rust,no_run
use alloy::{primitives::Address, rpc::types::TransactionRequest};
use kohaku_stealth::{
    Deployment, MetaAddress, contracts::prepare_announcement, generate_stealth_address,
};

fn example(
    deployment: &Deployment,
    recipient: &MetaAddress,
) -> Result<(Address, TransactionRequest), kohaku_stealth::SchemeError> {
    let mut rng = rand::rng();
    let generated = generate_stealth_address(recipient, &mut rng)?;
    let address = Address::from(generated.stealth_address);
    let transaction = prepare_announcement(deployment, &generated);
    Ok((address, transaction))
}
```

Sign and send `transaction` with Alloy. After it succeeds, fund `address`.
If funding fails, retry to the same address. Save each transaction hash before
waiting for a receipt. After a timeout, check its status before resending.

## Scanning

Create a scanner with `Account::scanner`. To scan with only the tracking key and
meta-address, import `StealthScheme` and call `SchemeId3::bind`.

```rust,no_run
use alloy::{
    providers::ProviderBuilder,
    rpc::client::RpcClient,
    transports::layers::RetryBackoffLayer,
};
use futures::{TryStreamExt, pin_mut};
use kohaku_stealth::{
    Deployment, ScanEvent, Scanner, StreamedEventConfig, get_announcements,
    get_announcements_for_user,
};

async fn receive(
    rpc_url: &str,
    deployment: Deployment,
    scanner: &Scanner,
    from_block: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = RpcClient::builder()
        .layer(RetryBackoffLayer::new(3, 250, 1_000))
        .http(rpc_url.parse()?);
    let provider = ProviderBuilder::new().connect_client(client);
    let announcements = get_announcements(
        &provider,
        deployment,
        from_block..=u64::MAX,
        StreamedEventConfig {
            max_retries: 0,
            ..Default::default()
        },
    );
    let events = get_announcements_for_user(scanner, announcements);
    pin_mut!(events);

    while let Some(event) = events.try_next().await? {
        match event {
            ScanEvent::Match(matched) => {
                println!("{} in {}", matched.stealth_address(), matched.record.transaction_hash);
            }
            ScanEvent::RangeScanned { range, rejected_logs } => {
                println!("scanned {range:?}; rejected {rejected_logs} malformed payloads");
            }
        }
    }
    Ok(())
}
```

Scanning stops at the finalized block. Start at the announcer's deployment block
unless you know when the recipient's keys were first used. Check funding separately.

Save matches before recording the completed range from `RangeScanned`.
Rescan unfinished ranges and skip logs already saved.

The example uses Alloy for retries. Keep `StreamedEventConfig.max_retries = 0`
on browsers. Native retries need a Tokio runtime with time enabled. Retry limits
apply per RPC request; failed ranges may be split and queried again.

## Keys

Store `AccountSeed` in a wallet keystore. Restore the account with
`Account::from_seed`, then derive a spend key from a matched address.
With Alloy's `signer-local` feature:

```rust,no_run
use alloy::signers::local::PrivateKeySigner;
use kohaku_stealth::{Account, ExposeSecret, MatchedStealthAddress};

fn example(
    account: &Account,
    matched: &MatchedStealthAddress,
) -> Result<PrivateKeySigner, Box<dyn std::error::Error>> {
    let key = account.spend_key(&matched.match_result)?;
    let signer = PrivateKeySigner::from_slice(key.expose_secret())?;
    Ok(signer)
}
```

`AccountSeed` and `SpendKey` clear their buffers on drop. A signer may keep its
own copy of the key.

`SchemeId3::announce` takes a 64-byte seed. Reusing a seed repeats
the ephemeral key and, for the same recipient, the stealth address. Use
`generate_stealth_address` with an RNG for new payments.

Import `StealthScheme` to call `SchemeId3` methods such as `bind`, `scan`,
`spend_key` and `meta_from_bytes`. Methods returning `Result` use `EngineError`;
account and RNG helpers use `SchemeError`.

## Tests

See [tests/README.md][tests] for native and browser commands, fixture sources,
and update instructions. Set `RUST_LOG=kohaku_stealth=debug` for SDK logs.

[tests]: https://github.com/0xakk0r0kamui/kohaku-sapq-rs/blob/master/crates/stealth/tests/README.md
