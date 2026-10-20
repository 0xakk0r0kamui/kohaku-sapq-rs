//! Signature-based keychain built.
//!
//! One EIP-712 signature per `(nonce, chain_id)` — the domain binds the chain, not a pool.
//! `trunc31(keccak256(tag || pool || rs))` then mixes the pool in, where `rs` is the signature's
//! big-endian `r` and low-`s` (32 bytes each) and `v` is dropped. Pools on the same chain share
//! that signature, which speeds up recovery.

use std::collections::{HashMap, HashSet};

use alloy::{
    dyn_abi::Eip712Domain,
    primitives::keccak256,
    signers::{Signature, Signer},
    sol_types::eip712_domain,
};
use kohaku_tornadocash::{Field, Note, NoteString, Nullifier, Pool, Secret};

mod sol {
    use alloy::sol;

    sol! {
        struct TornadoCashNote {
            uint64 nonce;
        }
    }
}

/// Keychain built on an alloy signer.
pub struct Keychain<S: Signer + Send + Sync> {
    signer: S,
}

/// Public non-spendable values derived from a note
#[derive(Clone, Copy)]
pub struct NoteHashes {
    pub commitment: Field,
    pub nullifier_hash: Field,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KeychainError {
    #[error("signer error: {0}")]
    Signer(#[from] alloy::signers::Error),
}

fn domain(chain_id: u64) -> Eip712Domain {
    eip712_domain! {
        name: "TornadoCash Keychain",
        version: "1",
        chain_id: chain_id,
        salt: alloy::primitives::keccak256(b"tornado"),
    }
}

impl<S: Signer + Send + Sync> Keychain<S> {
    pub fn new(signer: S) -> Self {
        Self { signer }
    }

    /// Derives the commitment and nullifier hash for `nonce` in each of `pools`.
    ///
    /// Pools that share a chain share one signature.
    ///
    /// The resulting Vector will match the order of the pools provided.
    pub async fn note_hashes(
        &self,
        nonce: u64,
        pools: &[Pool],
    ) -> Result<Vec<NoteHashes>, KeychainError> {
        let notes = self.notes(nonce, pools).await?;
        Ok(notes
            .into_iter()
            .map(|note| NoteHashes {
                commitment: note.commitment(),
                nullifier_hash: note.nullifier_hash(),
            })
            .collect())
    }

    /// Derives the `NoteString` for a given pool and nonce.
    #[expect(
        clippy::missing_panics_doc,
        reason = "self.notes always returns an equal number of notes as pool"
    )]
    pub async fn note(&self, nonce: u64, pool: &Pool) -> Result<NoteString, KeychainError> {
        let notes = self.notes(nonce, std::slice::from_ref(pool)).await?;
        Ok(notes.into_iter().next().unwrap())
    }

    /// Derives the `NoteStrings` on all pools for a given nonce.
    ///
    /// The resulting Vector will match the order of the pools provided. Pools on different chains
    /// are signed separately, because `chain_id` is part of the EIP-712 domain.
    async fn notes(&self, nonce: u64, pools: &[Pool]) -> Result<Vec<NoteString>, KeychainError> {
        let mut signatures = HashMap::<u64, Signature>::new();
        let chain_ids: HashSet<u64> = pools.iter().map(|p| p.chain_id).collect();
        for chain_id in chain_ids {
            let signature = self
                .signer
                .sign_typed_data(&sol::TornadoCashNote { nonce }, &domain(chain_id))
                .await?;
            signatures.insert(chain_id, signature);
        }

        let notes = pools
            .iter()
            .map(|pool| {
                let signature = &signatures[&pool.chain_id];
                let secret = secret_from_signature(pool, signature);
                let nullifier = nullifier_from_signature(pool, signature);

                NoteString::from_pool(Note::new(nullifier, secret), pool)
            })
            .collect();

        Ok(notes)
    }
}

fn secret_from_signature(pool: &Pool, sig: &Signature) -> Secret {
    Secret::new(from_signature(b"secret", pool, sig))
}

fn nullifier_from_signature(pool: &Pool, sig: &Signature) -> Nullifier {
    Nullifier::new(from_signature(b"nullifier", pool, sig))
}

fn from_signature(domain: &[u8], pool: &Pool, sig: &Signature) -> [u8; 31] {
    let rs = canonical_rs(sig);
    let bytes = keccak256([domain, pool.address.as_slice(), rs.as_slice()].concat());
    let mut nullifier_bytes = [0u8; 31];
    nullifier_bytes.copy_from_slice(&bytes[..31]);

    nullifier_bytes
}

/// Big-endian `r ‖ low-s`. `v` is omitted so software and hardware signers agree.
fn canonical_rs(sig: &Signature) -> [u8; 64] {
    let sig = sig.normalized_s();
    let mut rs = [0u8; 64];
    rs[..32].copy_from_slice(&sig.r().to_be_bytes::<32>());
    rs[32..].copy_from_slice(&sig.s().to_be_bytes::<32>());
    rs
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy::{primitives::U256, signers::local::LocalSigner};

    use super::*;

    const POOL: Pool = Pool::ETHEREUM_ETHER_1;

    #[test]
    fn note_salt_is_tornado() {
        let expected = keccak256(b"tornado");
        assert_eq!(Some(expected), domain(1).salt);
    }

    #[tokio::test]
    async fn note_deterministic() {
        //? Hackery to get deterministic signer for testing.
        let signer = LocalSigner::from_bytes(&U256::from(42).into()).unwrap();
        let keychain = Keychain::new(signer);

        let note = keychain.note(0, &POOL).await.unwrap();
        let expected = "tornado-eth-1-1-0xd60c35760fb4cb9ab527bd2b5f282f8de1a2ac7637b6fe7bcdc2e8442f5b77840ddb1813887350cee60b87c60badde4254f5fda056eaf9e6ff7ea1523745";
        assert_eq!(expected, note.to_string());
    }

    #[tokio::test]
    async fn nonce_and_pool_different() {
        let signer = LocalSigner::from_bytes(&U256::from(42).into()).unwrap();
        let keychain = Keychain::new(signer);

        let note1 = keychain.note(0, &POOL).await.unwrap();
        let note2 = keychain.note(1, &POOL).await.unwrap();
        let note3 = keychain.note(0, &Pool::ETHEREUM_ETHER_10).await.unwrap();

        assert_ne!(note1, note2);
        assert_ne!(note1, note3);
    }

    #[tokio::test]
    async fn high_s_and_v_do_not_change_the_preimage() {
        let signer = LocalSigner::from_bytes(&U256::from(42).into()).unwrap();
        let signature = signer
            .sign_typed_data(&sol::TornadoCashNote { nonce: 4 }, &domain(POOL.chain_id))
            .await
            .unwrap();
        let low = signature.normalized_s();
        let order =
            U256::from_str("0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141")
                .unwrap();
        let high = Signature::new(low.r(), order - low.s(), !low.v());
        let flipped_v = Signature::new(low.r(), low.s(), !low.v());

        assert_ne!(low.s(), high.s());
        for tag in [b"secret".as_slice(), b"nullifier".as_slice()] {
            let expected = from_signature(tag, &POOL, &low);
            assert_eq!(expected, from_signature(tag, &POOL, &high));
            assert_eq!(expected, from_signature(tag, &POOL, &flipped_v));
        }
    }
}
