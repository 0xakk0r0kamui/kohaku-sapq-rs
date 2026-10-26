# Tornado Merkle inclusion

Build a canonical Tornado tree from ordered commitments and obtain an inclusion
proof by index or commitment. Each proof retains the root captured at creation.

After loading the generated bindings as described in the
[build instructions](../README.md#build), use a nonempty caller-supplied
`commitments` array:

```js
const tree = MerkleTree.fromLeaves(commitments);
try {
  const proof = tree.leafProof(commitments[0]);
  try {
    console.log(proof.leaf, proof.root, proof.verify());
  } finally {
    proof.free();
  }
} finally {
  tree.free();
}
```
