# kohaku-tornadocash-wasm

JavaScript/TypeScript bindings for `kohaku-tornadocash`. This crate houses the
bindings for the core's public API, currently covering notes, assets, known pools,
deposit preparation and supplied-log decoding.
The wrappers follow the Rust API; wasm-bindgen and tsify stay in this crate.

## Available bindings

### Notes

- `new Note(nullifier, secret)` creates a note from two 31-byte hex strings.
- `new NoteString(note, symbol, amount, chainId)` adds metadata.
- `NoteString.parse(text)` parses a Tornado note; `toString()` formats it.
- Both classes expose read-only `nullifier` and `secret` getters, plus
  `preimage()`, `commitment()` and `nullifierHash()`.
- `NoteString` also exposes read-only `symbol`, `amount` and `chainId` getters.

Bytes use `0x`-prefixed hex strings, typed as `Hex` in TypeScript. Note outputs use lowercase hex.
`chainId` is a `bigint` in the `u64` range. Conversion and parsing errors throw
JavaScript `Error`s. All operations are synchronous.

`NoteString` consumes its input `Note`, even if chain ID validation fails.
Use the resulting `NoteString` afterward. Call `free()` on owned wrappers when
finished; consumed or freed wrappers must not be reused.

Example with fixed inputs, after generating the Node bindings:

```js
const { Note, NoteString } = require(
  './crates/target/tornadocash-wasm-node/kohaku_tornadocash_wasm.js',
);

const note = new Note(`0x${'01'.repeat(31)}`, `0x${'02'.repeat(31)}`);
const noteString = new NoteString(note, 'eth', '0.1', 1n);
const parsed = NoteString.parse(noteString.toString());

console.log(parsed.commitment());
parsed.free();
noteString.free();
```

### Known pools

- `Pool.known()` returns independently owned wrappers for every entry in the core's
  `Pool::POOLS` catalog.
- `Pool.fromNote(noteString)` borrows the note and finds a pool using its symbol,
  amount text and chain ID. The note remains usable after the lookup.
- `Pool.fromAddress(address)` finds a pool by its contract address, without a
  chain-ID parameter. The input is a typed `Hex` string and must decode to 20 bytes.
  Input checksum validation is not required.

Lookups delegate to the core and preserve its matching rules, including exact
amount text matching. A valid lookup without a match returns `undefined`;
malformed address input throws a JavaScript `Error`.

Read-only properties expose the pool metadata:

| Property | TypeScript type | Meaning |
| --- | --- | --- |
| `chainId` | `bigint` | The chain ID, from the core's `u64`. |
| `address` | `Hex` | The checksummed Tornado pool contract address. |
| `asset` | `Asset` | A new, independently owned wrapper for the pool's asset. |
| `amountWei` | `bigint` | The fixed deposit amount in asset base units, from the core's `u128`. |
| `deployedBlock` | `bigint` | The deployment block, from the core's `u64`. |

The integer properties retain their exact values. `amountWei` is the denomination
of each deposit, not the pool's balance. For example, the 1000 DAI pool's amount is
`1000000000000000000000n`. The asset's ERC20 token address and the pool's contract
address identify different contracts.

`id()`, `symbol()`, `amount()` and `toString()` delegate to the core and return
strings. Catalog reads and lookups are synchronous and do not query the blockchain.
Finding a pool from a note does not establish whether the note was deposited or spent.

Call `free()` on every returned Pool wrapper, including each entry from `known()`.
Each read of `pool.asset` creates a separate owned Asset wrapper; retain and free
that wrapper too. Freeing the Pool does not invalidate an Asset returned from it.
Metadata reads and `fromNote` borrow their inputs without consuming them.

Example with fixed inputs, after generating the Node bindings:

```js
const { NoteString, Pool } = require(
  './crates/target/tornadocash-wasm-node/kohaku_tornadocash_wasm.js',
);

const note = NoteString.parse(
  `tornado-eth-0.1-1-0x${'01'.repeat(31)}${'02'.repeat(31)}`,
);
let pool;
let asset;
try {
  pool = Pool.fromNote(note);
  if (pool === undefined) {
    throw new Error('No known pool matches this note');
  }

  asset = pool.asset;
  console.log(pool.id(), pool.amount(), pool.chainId, pool.amountWei);
  // eth-0.1-1 0.1 1n 100000000000000000n
  console.log(asset.kind, asset.symbol, asset.decimals);
  // native eth 18

  const byAddress = Pool.fromAddress(pool.address);
  try {
    console.log(byAddress?.id()); // eth-0.1-1
  } finally {
    byAddress?.free();
  }
  console.log(note.toString()); // The borrowed note remains usable.
} finally {
  asset?.free();
  pool?.free();
  note.free();
}
```

### Known assets

- `Asset.eth()` returns the core's native ETH asset.
- `Asset.matic()` returns the core's native MATIC asset.
- `Asset.ethereumDai()` returns the core's Ethereum DAI ERC20 asset.

Each factory returns a new, independently owned WASM wrapper. Read-only properties
expose the core metadata:

| Property | TypeScript type | Meaning |
| --- | --- | --- |
| `kind` | `string` | `"native"` or `"erc20"`. |
| `symbol` | `string` | The core's asset symbol. |
| `decimals` | `number` | The number of decimal places. |
| `address` | `Hex \| undefined` | The checksummed ERC20 token address; `undefined` for native assets. |

Use the named factories to obtain known assets. Call `free()` on each wrapper when
finished, and do not reuse a freed wrapper. Freeing one wrapper does not invalidate
another, even if both represent the same asset. Property reads borrow the wrapper
and do not consume it.

Example after generating the Node bindings:

```js
const { Asset } = require(
  './crates/target/tornadocash-wasm-node/kohaku_tornadocash_wasm.js',
);

const eth = Asset.eth();
const dai = Asset.ethereumDai();
try {
  console.log(eth.kind, eth.symbol, eth.decimals, eth.address);
  // native eth 18 undefined
  console.log(dai.kind, dai.symbol, dai.decimals, dai.address);
  // erc20 dai 18 0x6B175474E89094C44Da98b954EedeAC495271d0F
} finally {
  eth.free();
  dai.free();
}
```

### Custom assets

- `Asset.native(symbol, decimals)` creates native currency metadata.
- `Asset.erc20(address, symbol, decimals)` creates ERC20 token metadata.

Symbols are preserved as supplied, including case and whitespace. Decimals must
be a finite integer JavaScript `number` in `0..=255`; strings such as `"18"` are
rejected. ERC20 addresses use `Hex` and must decode to exactly 20 bytes; address
outputs use Alloy's canonical EIP-55 checksummed hex. Input checksum validation
is not required. Invalid inputs throw JavaScript `Error`s.

Each factory returns an independently owned wrapper with the same read-only
properties described above. Call `free()` on each wrapper when finished.
Custom construction leaves the core catalog and known-asset factories unchanged.

Example after generating the Node bindings:

```js
const { Asset } = require(
  './crates/target/tornadocash-wasm-node/kohaku_tornadocash_wasm.js',
);

const native = Asset.native('xyz', 6);
const token = Asset.erc20(
  '0x00000000000000000000000000000000000000AB',
  'ABC',
  18,
);
try {
  console.log(native.kind, native.symbol, native.decimals, native.address);
  // native xyz 6 undefined
  console.log(token.kind, token.symbol, token.decimals, token.address);
  // erc20 ABC 18 0x00000000000000000000000000000000000000AB
} finally {
  native.free();
  token.free();
}
```

### Deposit preparation

`Deposit` exposes `depositData()` and `approvalData()` to prepare plain `Call`
objects from a `Pool` and `Note`. See the [deposit guide](docs/deposits.md) for the
API, ownership and a short usage example.

### Event decoding

`decodeEvent` decodes caller-supplied Tornado logs into deposit or withdrawal
events. See the [event guide](docs/events.md) for a short usage example.

## Build

Run from the repository root. The wasm-bindgen CLI version must match the Rust dependency.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.108 --locked --bin wasm-bindgen

cargo build --locked --manifest-path crates/Cargo.toml \
  -p kohaku-tornadocash-wasm --target wasm32-unknown-unknown --target-dir crates/target

wasm-bindgen crates/target/wasm32-unknown-unknown/debug/kohaku_tornadocash_wasm.wasm \
  --target nodejs --out-dir crates/target/tornadocash-wasm-node

wasm-bindgen crates/target/wasm32-unknown-unknown/debug/kohaku_tornadocash_wasm.wasm \
  --target web --out-dir crates/target/tornadocash-wasm-web
```

Both targets generate TypeScript declarations. For the browser target, call the
module's default `init()` export before using the classes.

## Validation

The `WASM` GitHub Actions workflow builds WASM, runs strict Clippy, tests native
conversions and WASM bindings, and generates Node/browser bindings. It runs on
pull requests targeting `master`, pushes to `master`, and manual dispatch.

```sh
cargo clippy --locked --manifest-path crates/Cargo.toml \
  -p kohaku-tornadocash-wasm --target wasm32-unknown-unknown \
  --target-dir crates/target --no-deps -- -D warnings
```
