# kohaku-stealth

Rust support for ERC-5564 scheme 3. Announcement generation uses the protocol engine from [`pq-stealth-scheme3-public`](https://github.com/namnc/pq-stealth-scheme3-public) at revision `5fe8d0fd`. 

[`@kohaku-eth/pq-stealth-scheme3`](https://github.com/0xakk0r0kamui/kohaku-sapq/tree/pqsa-scheme3/crates/pq-stealth-ts) is the Kohaku plugin for the same scheme. 

Pass a provider configured with a funded signer, the deployment for its chain, and the recipient's validated meta-address. `resolve_meta_address` can read the latter from the ERC-6538 registry.

```rust,no_run
use alloy::{
    primitives::{Address, U256},
    providers::{DynProvider, Provider},
};
use kohaku_kv_store::Store;
use kohaku_stealth::{Deployment, StealthMetaAddress, StealthProvider};

async fn send_native_payment(
    rpc: DynProvider,
    deployment: Deployment,
    recipient: &StealthMetaAddress,
    amount: U256,
) -> Result<Address, Box<dyn std::error::Error>> {
    let provider = StealthProvider::rpc(&Store::create(), rpc.clone(), deployment);
    let mut rng = rand::rng();
    let payment = provider
        .payment(recipient)
        .native(amount)
        .prepare(&mut rng)?;

    let announcement = rpc
        .send_transaction(payment.announcement_transaction)
        .await?
        .get_receipt()
        .await?;
    if !announcement.status() {
        return Err("announcement transaction reverted".into());
    }

    let funding = rpc
        .send_transaction(payment.funding_transaction)
        .await?
        .get_receipt()
        .await?;
    if !funding.status() {
        return Err("funding transaction reverted".into());
    }

    Ok(payment.stealth_address)
}
```

`PreparedPayment` contains unsigned transactions. Wait for a successful announcement receipt before funding the address. If the announcement fails, do not send funds. If funding fails after a successful announcement, retry funding the same address; a new call to `prepare` generates a different payment. If a receipt request times out, check the transaction's chain status before retrying to avoid sending twice. Applications that require finality should wait for their chain's confirmation policy before funding.

Persist the recipient's `AccountSeed` in a wallet keystore before dropping it; the account can be reconstructed with `Scheme3Account::from_seed`. `PaymentBuilder::prepare` requires a `CryptoRng`. Deterministic vectors and applications with an external nonce protocol can use `scheme3::generate_stealth_address_with_seed`. An announcement seed must never be reused: reuse repeats the ephemeral key, and reuse for the same recipient also repeats the stealth address.

`StealthPrivateKey` erases its exported scalar on drop and redacts `Debug`. Call `expose_secret` only at the signer boundary.

## Logging

```bash
RUST_LOG=kohaku_stealth=debug cargo test -p kohaku-stealth -- --nocapture
```

## Tests

```bash
cargo fmt -p kohaku-stealth -- --check
cargo clippy -p kohaku-stealth --all-targets -- -D warnings
cargo test -p kohaku-stealth
```

The unit tests pin published vector V3-09 from the engine revision. The Anvil flow registers a meta-address, prepares and sends a payment, indexes announcement logs, scans a match, derives the one-time key, and spends from the resulting address.
