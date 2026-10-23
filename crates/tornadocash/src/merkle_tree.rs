//! Tornado Cash Merkle Tree instance.

use std::sync::OnceLock;

use alloy::primitives::keccak256;
use ark_bn254::Fr;
use ark_ff::PrimeField;
use kohaku_merkle_tree::{MerkleTreeError, hasher::Hasher};

use crate::{
    crypto::mimc::mimc_sponge_hash,
    field::Field,
    syncer::{Deposit, SyncEvent},
};

/// `MerkleTree` type used in Tornado Cash.
pub type MerkleTree = kohaku_merkle_tree::MerkleTree<20, 2, Field, TornadoHasher>;

/// `MerkleProof` type used in Tornado Cash.
pub type MerkleProof = kohaku_merkle_tree::proof::MerkleProof<20, 2, Field>;

/// `MerkleTree` extension trait for Tornado Cash.
pub trait MerkleTreeExt {
    /// Splices the given events into the Merkle tree.
    ///
    /// # Errors
    /// Returns an error if the events are non-contiguous or if the splicing fails.
    fn splice_events(&mut self, events: &[SyncEvent]) -> Result<(), MerkleTreeError>;
}

/// `MiMC` sponge hasher for Tornado Cash Merkle tree.
///
/// Reference implementation:
/// [contracts/Classic/MerkleTreeWithHistory.sol](https://github.com/tornado-dao/tornado-contracts/blob/cc57528ae13e762c36ae715f37e018528d6e0605/contracts/Classic/MerkleTreeWithHistory.sol#L57)
#[derive(Copy, Clone)]
pub struct TornadoHasher;

impl Hasher<2, Field> for TornadoHasher {
    fn hash(children: [Field; 2]) -> Field {
        mimc_sponge_hash(children[0].into(), children[1].into()).into()
    }

    fn zero() -> Field {
        static ZERO: OnceLock<Field> = OnceLock::new();
        *ZERO.get_or_init(|| {
            let hash = keccak256(b"tornado");
            Fr::from_be_bytes_mod_order(hash.as_slice()).into()
        })
    }
}

impl MerkleTreeExt for MerkleTree {
    fn splice_events(&mut self, events: &[SyncEvent]) -> Result<(), MerkleTreeError> {
        let deposits: Vec<&Deposit> = events
            .iter()
            .filter_map(|e| match e {
                SyncEvent::Deposit(d) => Some(d),
                SyncEvent::Withdrawal(_) => None,
            })
            .collect();

        let Some(first_leaf_index) = deposits.first().map(|d| d.leaf_index) else {
            return Ok(());
        };

        for (offset, d) in deposits.iter().enumerate() {
            let expected = first_leaf_index + offset as u32;
            if d.leaf_index != expected {
                return Err(MerkleTreeError::Other(format!(
                    "non-contiguous deposit events: expected leaf {expected}, got {}",
                    d.leaf_index
                )));
            }
        }

        let commitments: Vec<Field> = deposits.into_iter().map(|d| d.commitment).collect();
        self.splice(first_leaf_index as usize, &commitments)
    }
}

#[cfg(test)]
mod tests {
    use std::array::from_fn;

    use alloy::primitives::U256;
    use ruint::uint;

    use super::*;

    #[test]
    fn zero_hash_is_deterministic() {
        // https://etherscan.io/address/0x910cbd523d972eb0a6f4cae4618ad62622b39dbf#readContract#F17
        let zero: U256 = TornadoHasher::zero().into();
        let expected = uint!(
            21663839004416932945382355908790599225266501822907911457504978515578255421292_U256
        );

        assert_eq!(zero, expected);
    }

    #[test]
    fn hash_is_deterministic() {
        // https://etherscan.io/address/0x910cbd523d972eb0a6f4cae4618ad62622b39dbf#readContract#F3
        let l = uint!(0x0000000000000000000000000000000000000000000000000000000000000001_U256)
            .try_into()
            .unwrap();
        let r = uint!(0x0000000000000000000000000000000000000000000000000000000000000002_U256)
            .try_into()
            .unwrap();

        let hash: U256 = TornadoHasher::hash([l, r]).into();
        let expected = uint!(
            19814528709687996974327303300007262407299502847885145507292406548098437687919_U256
        );

        assert_eq!(hash, expected);
    }

    #[test]
    fn splice_is_deterministic() {
        let mut tree = crate::merkle_tree::MerkleTree::new();

        let leaves: [Field; 10] = from_fn(|i| Field::from(i + 1));
        tree.splice(0, &leaves).unwrap();

        let root: U256 = tree.root().unwrap().into();
        let expected = uint!(
            15200063891796499502721825879098395282261979630510984497744981505913469850275_U256
        );
        assert_eq!(root, expected);
    }
}
