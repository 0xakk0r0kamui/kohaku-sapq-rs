use std::fmt::Display;

use alloy::primitives::{Address, B256, Bytes, TxHash};
use serde::{Deserialize, Serialize};

/// A job on a relayer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobId(String);

/// A withdrawal request.
///
/// Request to `v1/tornadoWithdraw`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawRequest {
    pub contract: Address,
    pub proof: Bytes,
    /// (`root`, `nullifier_hash`, `recipient`, `relayer`, `fee`, `refund`).
    ///
    /// Uses B256 for `fee` and `refund` so they are serialized as 32-byte hex strings.
    pub args: (B256, B256, Address, Address, B256, B256),
}

/// A withdrawal job.
///
/// Response from `v1/tornadoWithdraw`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawResponse {
    pub id: JobId,
}

/// A withdrawal job's status.
///
/// Response from `v1/jobs/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResponse {
    pub status: JobStatus,
    pub tx_hash: Option<TxHash>,
    pub confirmations: Option<u32>,
    pub failed_reason: Option<String>,
}

/// A job's status on a relayer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobStatus {
    Queued,
    Accepted,
    Sent,
    Mined,
    Resubmitted,
    Confirmed,
    Failed,
}

impl Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
