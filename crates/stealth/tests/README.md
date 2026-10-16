# Tests

## Native

Run from `crates/`, with Anvil on `PATH`:

```sh
cargo test --locked -p kohaku-stealth
cargo clippy --locked -p kohaku-stealth --all-targets -- -D warnings
cargo fmt -p kohaku-stealth -- --check
```

`e2e.rs` installs ERC-5564/6538 runtime bytecode on Anvil. Contract deployment
and constructors are not tested.

## Firefox

Install Firefox and `wasm-pack`, then run from the repo root:

```sh
rustup target add wasm32-unknown-unknown
wasm-pack test --headless --firefox crates/stealth --test interop --locked
```

`interop.rs` runs on native and in Firefox. RPC is mocked; browser HTTP and CORS
are not tested.

## Fixtures

The sources below are pinned to [pq-stealth-scheme3-public at b576b37c][upstream],
the revision used by `pqsa-*` in the workspace manifest.

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

Run in the upstream workspace at the same revision as `pqsa-*`:

```sh
cargo run --locked -p pqsa-per-payment --example emit_payment_json
```

Save stdout to `crates/stealth/tests/fixtures/payment.json` in this repo.

[upstream]: https://github.com/namnc/pq-stealth-scheme3-public/tree/b576b37cbd80db51c343b85d5fdad599b298047d
[announcer]: https://github.com/namnc/pq-stealth-scheme3-public/blob/b576b37cbd80db51c343b85d5fdad599b298047d/harness/announcement/deployed_bytecode.hex
[registry]: https://github.com/namnc/pq-stealth-scheme3-public/blob/b576b37cbd80db51c343b85d5fdad599b298047d/harness/registration/deployed_bytecode.hex
[payment]: https://github.com/namnc/pq-stealth-scheme3-public/blob/b576b37cbd80db51c343b85d5fdad599b298047d/crates/per-payment/examples/emit_payment_json.rs
[keygen]: https://github.com/namnc/pq-stealth-scheme3-public/blob/b576b37cbd80db51c343b85d5fdad599b298047d/crates/core/src/lib.rs
[vectors]: https://github.com/namnc/pq-stealth-scheme3-public/blob/b576b37cbd80db51c343b85d5fdad599b298047d/vectors/section-2.json
