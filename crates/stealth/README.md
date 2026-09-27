# kohaku-stealth

Rust client for ERC-5564 schemeId 3. The announcement is post-quantum. Spending is an ordinary secp256k1 key.

The scheme operations are `SchemeId3` from [`pq-stealth-scheme3-public`](https://github.com/namnc/pq-stealth-scheme3-public) at `5fe8d0fd`. This crate does not implement ML-KEM, ECDH, or the key schedule. It stores the sender counter, reads `Announcement` logs, and returns unsigned transactions.

[`@kohaku-eth/pq-stealth-scheme3`](https://github.com/0xakk0r0kamui/kohaku-sapq/tree/pqsa-scheme3/crates/pq-stealth-ts) is the Kohaku plugin for the same scheme. 

## Example

```rust,no_run
use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use kohaku_kv_store::Store;
use kohaku_stealth::{Asset, Deployment, Scheme3Client, Transfer, keygen, spend_key};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ProviderBuilder::new()
        .connect_http("http://localhost:8545".parse()?)
        .erased();
    let deployment = Deployment {
        announcer: Address::ZERO,
        registry: Address::ZERO,
        start_block: 0,
        scheme_id: kohaku_stealth::SCHEME_ID,
    };
    let client = Scheme3Client::new(&Store::create(), provider, deployment)?;

    let keys = keygen(&[0u8; 128])?;
    let _register = client.register_keys_tx(&keys.meta_address);
    let payment = client
        .prepare_payment(
            &keys.meta_address,
            [0x42; 32],
            Transfer {
                asset: Asset::Native,
                amount: U256::from(1),
            },
        )
        .await?;
    client.sync().await?;
    let matches = client.scan(&keys.tracking, &keys.meta_address).await?;
    let _scalar = spend_key(&keys.master, &matches[0])?;
    let _ = payment.announce;
    Ok(())
}
```

## Test

```bash
cargo test -p kohaku-stealth
cargo clippy -p kohaku-stealth --all-targets -- -D warnings
```

The unit tests pin published vector V3-09 from that engine revision: the same 128-byte seed produces that meta-address, a zero scalar and the curve order are rejected, and `n - 1` is accepted. 

`tests/flow.rs` runs Anvil. One test checks that a second `Scheme3Client` on the same store uses the next sender index. The other registers, announces, funds, writes a scheme-3 log with the wrong shape, writes a scheme-4 log, syncs, scans exactly one match, and signs a transfer with `alloy` using `spend_key`. `RUST_LOG=kohaku_stealth=debug` prints the lengths, the stealth address, the sender index, and the skip.