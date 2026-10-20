use alloy::{primitives::Address, sol_types::SolStruct};
use serde::{Deserialize, Serialize};

use crate::{
    abis::entry_point::EntryPoint::PackedUserOperation,
    entry_point::entry_point_08_domain,
    user_operation::{UserOperation, UserOperationHash},
};

/// A signed 4337 UserOperation with an optional signed 7702 Authorization
/// ready to be sent to a bundler.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(js, derive(tsify::Tsify))]
#[cfg_attr(js, tsify(into_wasm_abi, from_wasm_abi))]
#[serde(rename_all = "camelCase")]
pub struct SignedUserOperation {
    pub user_op: UserOperation,
    pub entry_point: Address,
}

impl SignedUserOperation {
    /// EIP-712 hash of this operation for EntryPoint 0.8 on `chain_id`.
    ///
    /// A bundler that already has the operation still returns this hash, so the caller can poll
    /// for the receipt instead of submitting it again.
    #[must_use]
    pub fn hash(&self, chain_id: u64) -> UserOperationHash {
        UserOperationHash(
            PackedUserOperation::from(&self.user_op)
                .eip712_signing_hash(&entry_point_08_domain(chain_id)),
        )
    }
}
