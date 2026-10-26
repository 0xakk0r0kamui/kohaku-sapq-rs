use std::array::from_fn;

use alloy::{
    primitives::{Address, Bytes},
    sol_types::SolCall,
};
use ark_bn254::{
    G1Affine, G2Affine,
    g2::{G2_GENERATOR_X, G2_GENERATOR_Y},
};
use ark_ff::MontFp;
use kohaku_tornadocash_circuit::{CircuitInputs, prove};
use rand::CryptoRng;
use ruint::aliases::U256;
use websnark_rs::proof::Proof;

use crate::{
    Asset, Call, abis::tornado::Tornado::withdrawCall, field::Field, merkle_tree::MerkleProof,
    note::Note, pool::Pool,
};

/// A Tornado Cash withdrawal.
#[derive(Debug, Clone)]
pub struct Withdrawal {
    pub pool: Pool,
    pub note: Note,
    pub recipient: Address,
    pub payer: Payer,
}

/// A proven Tornado Cash withdrawal.
#[derive(Debug, Clone)]
pub struct ProvenWithdrawal {
    inner: Withdrawal,
    root: Field,
    proof: Proof,
}

/// The payer of a withdrawal transaction.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct Payer {
    pub address: Address,
    pub fee: U256,
    pub refund: U256,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WithdrawalError {
    #[error("Circuit error: {0}")]
    Circuit(#[from] kohaku_tornadocash_circuit::CircuitError),
    #[error("Proof leaf mismatch: expected {expected}, got {actual}")]
    ProofLeafMismatch { expected: Field, actual: Field },
    #[error("Nonzero refund for native-asset withdrawal: {0}")]
    NonzeroRefundForNativeAsset(U256),
}

impl Withdrawal {
    #[must_use]
    pub fn new(pool: &Pool, note: Note, recipient: Address) -> Self {
        Self {
            pool: pool.clone(),
            note,
            recipient,
            payer: Payer::default(),
        }
    }

    #[must_use]
    pub fn with_payer(mut self, payer: Payer) -> Self {
        self.payer = payer;
        self
    }

    /// Proves this withdrawal using the provided Merkle proof.
    ///
    /// `merkle_proof` must be a valid proof for this note's commitment against a
    /// recent Merkle root. Outdated proofs are rejected by the contract.
    ///
    /// # Errors
    /// Returns an error if refund is nonzero for native withdrawals, `merkle_proof` does not match
    /// this withdrawal's commitment, or if proving fails.
    pub fn prove(
        self,
        merkle_proof: &MerkleProof,
        rng: &mut impl CryptoRng,
    ) -> Result<ProvenWithdrawal, WithdrawalError> {
        let is_native = matches!(self.pool.asset, Asset::Native { .. });
        if is_native && !self.payer.refund.is_zero() {
            return Err(WithdrawalError::NonzeroRefundForNativeAsset(
                self.payer.refund,
            ));
        }

        if merkle_proof.leaf != self.note.commitment() {
            return Err(WithdrawalError::ProofLeafMismatch {
                expected: self.note.commitment(),
                actual: merkle_proof.leaf,
            });
        }

        let path_elements = merkle_proof.sibling_paths();
        let path_elements = from_fn(|i| path_elements[i].into());
        let path_indices = from_fn(|i| U256::from(merkle_proof.path[i]));

        let circuit_inputs = CircuitInputs::new(
            merkle_proof.root.into(),
            self.note.nullifier_hash().into(),
            self.recipient.into_word().into(),
            self.payer.address.into_word().into(),
            self.payer.fee,
            self.payer.refund,
            self.note.nullifier.into(),
            self.note.secret.into(),
            path_elements,
            path_indices,
        );

        let proof = prove(&circuit_inputs, rng)?;
        Ok(ProvenWithdrawal {
            root: merkle_proof.root,
            proof,
            inner: self,
        })
    }

    /// Builds this withdrawal with a dummy proof.
    ///
    /// For gas estimation only. The dummy proof is invalid and will only be accepted by an
    /// overridden verifier.
    #[must_use]
    pub fn prove_dummy(self, merkle_proof: &MerkleProof) -> ProvenWithdrawal {
        ProvenWithdrawal {
            root: merkle_proof.root,
            proof: dummy_proof(),
            inner: self,
        }
    }
}

impl ProvenWithdrawal {
    #[must_use]
    pub fn pool(&self) -> &Pool {
        &self.inner.pool
    }

    #[must_use]
    pub fn note(&self) -> &Note {
        &self.inner.note
    }

    #[must_use]
    pub fn recipient(&self) -> Address {
        self.inner.recipient
    }

    #[must_use]
    pub fn payer(&self) -> Payer {
        self.inner.payer
    }

    #[must_use]
    pub fn root(&self) -> Field {
        self.root
    }

    /// Returns the call to withdraw from the pool using the generated proof.
    ///
    /// ERC20 withdrawals attach the native-currency refund paid to the recipient.
    #[must_use]
    pub fn call(&self) -> Call {
        let input = withdrawCall {
            _proof: self.solidity_proof(),
            _root: self.root.into(),
            _nullifierHash: self.note().nullifier_hash().into(),
            _recipient: self.recipient(),
            _relayer: self.payer().address,
            _fee: self.payer().fee,
            _refund: self.payer().refund,
        }
        .abi_encode();

        Call {
            target: self.pool().address,
            value: self.payer().refund,
            input: input.into(),
        }
    }

    /// Returns the abi-encoded proof for this withdrawal transaction.
    #[must_use]
    pub fn solidity_proof(&self) -> Bytes {
        into_solidity_proof(&self.proof)
    }
}

impl From<ProvenWithdrawal> for Withdrawal {
    fn from(proven: ProvenWithdrawal) -> Self {
        proven.inner
    }
}

/// Convert a websnark proof into the format expected by the Solidity contract.
fn into_solidity_proof(proof: &Proof) -> Bytes {
    let proof_elements: [U256; 8] = [
        proof.a.x.into(),
        proof.a.y.into(),
        //? Order of b elements are reversed to match Solidity's expected format
        proof.b.x.c1.into(),
        proof.b.x.c0.into(),
        proof.b.y.c1.into(),
        proof.b.y.c0.into(),
        proof.c.x.into(),
        proof.c.y.into(),
    ];
    let mut proof_bytes = Vec::with_capacity(256);
    for elem in &proof_elements {
        proof_bytes.extend_from_slice(&elem.to_be_bytes::<32>());
    }

    proof_bytes.into()
}

/// Non-zero dummy proof for gas estimation.
fn dummy_proof() -> Proof {
    // The point 2*G1
    let two_g1 = G1Affine::new_unchecked(
        MontFp!("1368015179489954701390400359078579693043519447331113978918064868415326638035"),
        MontFp!("9918110051302171585080402603319702774565515993150576347155970296011118125764"),
    );
    Proof {
        a: two_g1,
        b: G2Affine::new_unchecked(G2_GENERATOR_X, G2_GENERATOR_Y),
        c: two_g1,
    }
}

#[cfg(test)]
mod tests {
    use alloy::primitives::B256;
    use ark_bn254::{G1Affine, G2Affine};

    use super::*;

    #[test]
    fn withdrawal_call_maps_parameters_and_attaches_erc20_refund() {
        for (pool, refund) in [
            (Pool::ETHEREUM_ETHER_01, U256::ZERO),
            (Pool::ETHEREUM_DAI_100, U256::from(42)),
        ] {
            let withdrawal = ProvenWithdrawal {
                root: Field::from(7u32),
                // Call encoding only needs proof coordinates; no proof generation is exercised.
                proof: Proof {
                    a: G1Affine::default(),
                    b: G2Affine::default(),
                    c: G1Affine::default(),
                },
                inner: Withdrawal::new(
                    &pool,
                    Note::new([1; 31], [2; 31]),
                    Address::with_last_byte(1),
                )
                .with_payer(Payer {
                    address: Address::with_last_byte(2),
                    fee: U256::from(3),
                    refund,
                }),
            };
            let call = withdrawal.call();
            let input = withdrawCall::abi_decode(&call.input).unwrap();

            assert_eq!(call.target, pool.address);
            assert_eq!(call.value, refund);
            assert_eq!(input._proof, withdrawal.solidity_proof());
            assert_eq!(input._root, B256::from(withdrawal.root));
            assert_eq!(
                input._nullifierHash,
                B256::from(withdrawal.note().nullifier_hash())
            );
            assert_eq!(input._recipient, withdrawal.recipient());
            assert_eq!(input._relayer, withdrawal.payer().address);
            assert_eq!(input._fee, withdrawal.payer().fee);
            assert_eq!(input._refund, refund);
        }
    }
}
