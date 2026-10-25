# kohaku-tornadocash-keychain

Deterministic note derivation for [`kohaku-tornadocash`](../core/).

[`Keychain`]s are used to derive tornadocash note secrets from a single source of entropy. This makes it easier for users to manage their notes, because they only need to backup a single secret (e.g. a mnemonic) instead of each individual note secret. New notes can be derived from the keychain by incrementing a nonce, and notes can be recovered by scanning pools' synced events against the keychain's derivation scheme.

## Nonce Hygiene

Nonces are used to derive tornadocash note secrets from a keychain. Nonces should:
- Be monotonically increasing, generally starting from 0. Increasing the nonce by too much can result in unrecoverable notes if the gap limit is exceeded.
- Be unique across all pools. [`next_nonce`] returns one past the highest nonce used in any pool. Recovery checks every nonce against every pool, so a nonce reused in a different pool is still recovered, but reusing a nonce in the same pool results in an invalid note that cannot be deposited. This may result in compromised privacy, linking multiple addresses to the same note.

## Examples

### Deposit

```rust,no_run
use alloy::signers::Signer;
use kohaku_tornadocash::{Deposit, Pool};
use kohaku_tornadocash_keychain::Keychain;

async fn example<S: Signer + Send + Sync>(
    keychain: &Keychain<S>,
    pool: &Pool,
    nonce: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let note = keychain.note(nonce, pool).await?;
    let deposit = Deposit::new(pool, note.note);

    Ok(())
}
```

### Withdraw

```rust,no_run
use alloy::signers::Signer;
use kohaku_tornadocash::{Pool, Withdrawal};
use kohaku_tornadocash_keychain::Keychain;

async fn example<S: Signer + Send + Sync>(
    keychain: &Keychain<S>,
    pool: &Pool,
    nonce: u64,
    recipient: alloy::primitives::Address,
) -> Result<(), Box<dyn std::error::Error>> {
    let note = keychain.note(nonce, pool).await?;
    let withdrawal = Withdrawal::new(pool, note.note, recipient);

    Ok(())
}
```

### Note Recovery

```rust,no_run
use alloy::signers::Signer;
use kohaku_tornadocash::event::Snapshot;
use kohaku_tornadocash_keychain::{Keychain, next_nonce, recover};

async fn example<S: Signer + Send + Sync>(
    keychain: &Keychain<S>,
    batches: &[Snapshot],
) -> Result<(), Box<dyn std::error::Error>> {
    let notes = recover(keychain, batches, None).await?;
    for note in &notes {
        println!("{} ({}): leaf {}", note.nonce, note.pool, note.deposit.leaf_index);
    }

    let next_nonce = next_nonce(keychain, batches, None).await?;
    // Or calculate manually:
    // let next_nonce = notes.last().map(|note| note.nonce + 1).unwrap_or(0);
    Ok(())
}
```
