use std::{array::from_fn, ops::Deref};

use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes},
    rpc::types::TransactionRequest,
    sol_types::SolCall,
};
use kohaku_tornadocash_circuit::{CircuitInputs, prove};
use rand::CryptoRng;
use ruint::aliases::U256;
use websnark_rs::proof::Proof;

use crate::{
    abis::tornado::Tornado::withdrawCall, field::Field, merkle_tree::MerkleProof, note::Note,
    pool::Pool,
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
    pub root: Field,
    pub proof: Proof,
    pub inner: Withdrawal,
}

/// The payer of a withdrawal transaction.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct Payer {
    pub address: Address,
    pub fee: U256,
    pub refund: U256,
}

#[derive(Debug, thiserror::Error)]
pub enum WithdrawalError {
    #[error("Circuit error: {0}")]
    Circuit(#[from] kohaku_tornadocash_circuit::CircuitError),
    #[error("Proof leaf mismatch: expected {expected}, got {actual}")]
    ProofLeafMismatch { expected: Field, actual: Field },
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
    /// Returns an error if `merkle_proof` does not match this withdrawal's commitment,
    /// or if proving fails.
    pub fn prove(
        self,
        merkle_proof: &MerkleProof,
        rng: &mut impl CryptoRng,
    ) -> Result<ProvenWithdrawal, WithdrawalError> {
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
}

impl ProvenWithdrawal {
    /// Returns the input data for this withdrawal transaction.
    #[must_use]
    pub fn input(&self) -> Vec<u8> {
        withdrawCall {
            _proof: self.solidity_proof(),
            _root: self.root.into(),
            _nullifierHash: self.note.nullifier_hash().into(),
            _recipient: self.recipient,
            _relayer: self.payer.address,
            _fee: self.payer.fee,
            _refund: self.payer.refund,
        }
        .abi_encode()
    }

    /// Returns the target address for this withdrawal transaction.
    #[must_use]
    pub fn target(&self) -> Address {
        self.pool.address
    }

    /// Returns the abi-encoded proof for this withdrawal transaction.
    #[must_use]
    pub fn solidity_proof(&self) -> Bytes {
        into_solidity_proof(&self.proof)
    }
}

impl Deref for ProvenWithdrawal {
    type Target = Withdrawal;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl From<ProvenWithdrawal> for TransactionRequest {
    fn from(proven_withdrawal: ProvenWithdrawal) -> Self {
        TransactionRequest::default()
            .with_to(proven_withdrawal.target())
            .input(proven_withdrawal.input().into())
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
