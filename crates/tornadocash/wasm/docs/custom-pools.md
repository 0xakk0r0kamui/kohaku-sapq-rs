# Custom pools

Construct a pool with asset metadata and optional paymaster addresses. This Node
snippet uses the [generated bindings](../README.md#build) from the repository root.

```js
const { Asset, Pool } = require(
  './crates/target/tornadocash-wasm-node/kohaku_tornadocash_wasm.js',
);

const asset = Asset.native('XYZ', 6);
let pool;
try {
  pool = new Pool(
    1n,
    '0x00000000000000000000000000000000000000ef',
    asset, 1230000n, 19000000n,
    {
      address: '0x00000000000000000000000000000000000000ab',
      adapter: '0x00000000000000000000000000000000000000cd',
    },
  );
  console.log(pool.amount(), pool.paymaster);
} finally {
  pool?.free();
  asset.free(); // The constructor borrows the Asset and keeps its own metadata.
}
```

Omit the final argument for a pool without paymaster metadata. `pool.paymaster`
is a plain object or `undefined`, and requires no `free()` call.
