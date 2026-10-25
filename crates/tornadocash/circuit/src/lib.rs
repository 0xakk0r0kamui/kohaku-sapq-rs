#![doc = include_str!("../README.md")]

use rand::CryptoRng;
use websnark_rs::proof::{Proof, prove_random};

use crate::artifacts::{circuit, proving_key};

pub mod artifacts;
mod inputs;

pub use inputs::CircuitInputs;

#[derive(Debug, thiserror::Error)]
pub enum CircuitError {
    #[error("Circuit error: {0}")]
    Circuit(#[from] websnark_rs::circuit::CircuitError),
    #[error("Proof generation error: {0}")]
    Proof(#[from] websnark_rs::proof::ProofError),
    #[error("Field conversion error: {0}")]
    ToField(#[from] ruint::ToFieldError),
}

/// Generates a proof for the given circuit inputs.
///
/// # Errors
/// Returns an error if the inputs are invalid or if the proof generation fails.
pub fn prove(inputs: &CircuitInputs, rng: &mut impl CryptoRng) -> Result<Proof, CircuitError> {
    let circuit = circuit();
    let proving_key = proving_key();

    let signals = inputs.as_signals()?;
    let witness = circuit.witness(signals)?;
    let (proof, _) = prove_random(proving_key, &witness, rng)?;
    Ok(proof)
}
