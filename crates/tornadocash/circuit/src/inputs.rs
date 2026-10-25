use std::collections::HashMap;

use ruint::{ToFieldError, aliases::U256};
use serde::{Deserialize, Serialize};
use websnark_rs::circuit::Value;

/// Tornadocash circuit inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CircuitInputs {
    /// Merkle root for the inclusion proof.
    root: U256,
    nullifier_hash: U256,
    recipient: U256,
    relayer: U256,
    fee: U256,
    refund: U256,
    nullifier: U256,
    secret: U256,
    /// Sibling path elements for the merkle inclusion proof.
    path_elements: [U256; 20],
    /// This note's path indices for the merkle inclusion proof.
    path_indices: [U256; 20],
}

impl CircuitInputs {
    #[allow(clippy::too_many_arguments, clippy::large_types_passed_by_value)]
    #[must_use]
    pub fn new(
        root: U256,
        nullifier_hash: U256,
        recipient: U256,
        relayer: U256,
        fee: U256,
        refund: U256,
        nullifier: U256,
        secret: U256,
        path_elements: [U256; 20],
        path_indices: [U256; 20],
    ) -> Self {
        Self {
            root,
            nullifier_hash,
            recipient,
            relayer,
            fee,
            refund,
            nullifier,
            secret,
            path_elements,
            path_indices,
        }
    }

    /// Convert the circuit inputs into a circuit input signal map
    ///
    /// # Errors
    /// Returns an error if any of the inputs are not valid field elements.
    pub fn as_signals(&self) -> Result<HashMap<String, Value>, ToFieldError> {
        Ok(HashMap::from([
            ("root".into(), to_value(self.root)?),
            ("nullifierHash".into(), to_value(self.nullifier_hash)?),
            ("recipient".into(), to_value(self.recipient)?),
            ("relayer".into(), to_value(self.relayer)?),
            ("fee".into(), to_value(self.fee)?),
            ("refund".into(), to_value(self.refund)?),
            ("nullifier".into(), to_value(self.nullifier)?),
            ("secret".into(), to_value(self.secret)?),
            ("pathElements".into(), to_value_arr(&self.path_elements)?),
            ("pathIndices".into(), to_value_arr(&self.path_indices)?),
        ]))
    }
}

fn to_value(value: U256) -> Result<Value, ToFieldError> {
    Ok(Value::Fr(value.try_into()?))
}

fn to_value_arr(values: &[U256]) -> Result<Value, ToFieldError> {
    Ok(Value::Array(
        values
            .iter()
            .map(|v| to_value(*v))
            .collect::<Result<_, _>>()?,
    ))
}
