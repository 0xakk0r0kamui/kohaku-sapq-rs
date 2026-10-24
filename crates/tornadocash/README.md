# kohaku-tornadocash

Rust [Tornadocash](https://tornadocash.eth.limo/) client library, designed to interface with Tornado Cash's smart contracts. It provides support for:
- Deposit and withdrawal transaction generation
- Pluggable event syncing (JSON-RPC, cached remote, saga-sync)
- Merkle tree construction and proof generation
- Relayed and paymaster-sponsored withdrawal transactions

`Deposit::deposit_data()`, `Deposit::approval_data()` and `ProvenWithdrawal::data()`
prepare `Call` values with a target address, native value and encoded input.
Native deposits have no approval call. A `Call` can be converted into Alloy's
`TransactionRequest` for signing and submission.

## Examples

### Deposit

```rust,no_run
# use rand::RngExt;
# use alloy::providers::{DynProvider, Provider};
# use kohaku_tornadocash::{Deposit, Pool};
# 
# async fn example(
#     provider: DynProvider, 
#     rng: &mut impl rand::CryptoRng,
# ) -> Result<(), Box<dyn std::error::Error>> {
let deposit = Deposit::new(&Pool::SEPOLIA_ETHER_01, rng.random());
let note = deposit.note.clone();
    
// ERC20 pools require approval for at least the deposit amount.
if let Some(approval) = deposit.approval() {
    provider.send_transaction(approval.into()).await?.watch().await?;
}
provider.send_transaction(deposit.call().into()).await?.watch().await?;

# Ok(())
# }
```

### Withdrawal

```rust,no_run
# use alloy::{
#     primitives::Address,
#     providers::{DynProvider, Provider},
# };
# use kohaku_tornadocash::{
#     merkle_tree::{MerkleTree, MerkleTreeExt},
#     Note,
#     Pool,
#     syncer::{DynSyncer, Syncer},
#     Withdrawal,
# };
# 
# async fn example(
#     provider: DynProvider,
#     syncer: &DynSyncer,
#     pool: Pool,
#     note: Note,
#     recipient: Address,
#     rng: &mut impl rand::CryptoRng,
# ) -> Result<(), Box<dyn std::error::Error>> {
let mut tree = MerkleTree::new();
let snapshot = syncer.sync(&pool, ..).await?;
tree.splice_events(&snapshot.events)?;

let merkle_proof = tree.leaf_proof(note.commitment())?;
let withdrawal = Withdrawal::new(&pool, note, recipient)
    .prove(&merkle_proof, rng)?;

provider.send_transaction(withdrawal.call().into()).await?.watch().await?;

# Ok(())
# }
```
