# Tests

## Native

Run from `crates/`, with Anvil on `PATH`:

```sh
cargo test --locked -p kohaku-stealth -p kohaku-rpc-provider
cargo clippy --locked -p kohaku-stealth -p kohaku-rpc-provider --all-targets -- -D warnings
cargo fmt -p kohaku-stealth -p kohaku-rpc-provider -- --check
```

`e2e.rs` installs ERC-5564/6538 runtime bytecode on Anvil. Contract deployment
and constructors are not tested.

## Firefox

Install Firefox and `wasm-pack`, then run from the repo root:

```sh
rustup target add wasm32-unknown-unknown
wasm-pack test --headless --firefox crates/stealth --test interop --locked
```

`interop.rs` runs on native and in Firefox. Retry tests use Alloy's retry layer
with `StreamedEventConfig.max_retries = 0`. RPC responses are mocked; these tests
do not cover HTTP or CORS.

## Fixtures

The fixtures come from [pq-stealth-scheme3-public v0.1.0][upstream].

| Fixture or test | Upstream source |
| --- | --- |
| `contracts/ERC5564Announcer.hex` | [harness/announcement/deployed_bytecode.hex][announcer] |
| `contracts/ERC6538Registry.hex` | [harness/registration/deployed_bytecode.hex][registry] |
| `fixtures/payment.json` | Output of [emit_payment_json.rs][payment] |
| Keygen seed | [keygen_seed_golden][keygen] in `crates/core/src/lib.rs` |
| Token metadata in `interop.rs` | V3-08a, `given.metadata_with_token_block` in [vectors/section-2.json][vectors] |

The token metadata test keeps the payment fixture's view tag and appends the
remaining bytes from V3-08a.

### Regenerate payment.json

In `pq-stealth-scheme3-public`, check out the version used by `pqsa-*` and run:

```sh
cargo run --locked -p pqsa-per-payment --example emit_payment_json
```

Save stdout to `crates/stealth/tests/fixtures/payment.json` in this repo.

[upstream]: https://github.com/namnc/pq-stealth-scheme3-public/tree/v0.1.0
[announcer]: https://github.com/namnc/pq-stealth-scheme3-public/blob/v0.1.0/harness/announcement/deployed_bytecode.hex
[registry]: https://github.com/namnc/pq-stealth-scheme3-public/blob/v0.1.0/harness/registration/deployed_bytecode.hex
[payment]: https://github.com/namnc/pq-stealth-scheme3-public/blob/v0.1.0/crates/per-payment/examples/emit_payment_json.rs
[keygen]: https://github.com/namnc/pq-stealth-scheme3-public/blob/v0.1.0/crates/core/src/lib.rs
[vectors]: https://github.com/namnc/pq-stealth-scheme3-public/blob/v0.1.0/vectors/section-2.json
