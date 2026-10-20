# kohaku-merkle-tree

Generic Merkle Tree implemented in Rust. 

Exports a simple API for creating in-memory Merkle Trees, inserting leaves, and generating proofs. Merkle Trees are generic over the depth, arity, leaf type, and hash function.

## Examples

```rust
use kohaku_merkle_tree::{MerkleTree, hasher::Hasher};

# #[derive(Clone)]
# pub struct ToyHasher;
# 
# impl Hasher<2, u32> for ToyHasher {
#     fn hash(children: [u32; 2]) -> u32 {
#         children[0] + children[1]
#     }
# 
#     fn zero() -> u32 {
#         0
#     }
# }
# 
# fn example() -> Result<(), Box<dyn std::error::Error>> {
let mut tree = MerkleTree::<4, 2, u32, ToyHasher>::new();

tree.insert(0, 1);
tree.insert(1, 2);
tree.insert(2, 3);
tree.splice(3, &[4, 5, 6])?;

let proof = tree.proof(1)?;

# Ok(())
# }
```

## Features

- `parallel` - enables parallel hashing of Merkle Tree levels using the `rayon` crate.

## Benchmarks

Benchmarks were run on a Ryzen 5 3600, 32GB RAM.

| Method               | Target           | Time    |
| -------------------- | ---------------- | ------- |
| merkle_splice/100    | native           | 180 us  |
| merkle_splice/10_000 | native           | 13.4 ms |
| merkle_splice/10_000 | native -parallel | 2.28 ms |
| merkle_proof/100     | native           | 4.8 us  |
| merkle_proof/10_000  | native           | 480 us  |
