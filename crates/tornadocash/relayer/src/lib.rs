//! Client for interacting with Tornadocash relayers.
//!
//! Tornadocash relayers are services run by third parties that accept and submit withdrawal
//! proofs on behalf of users. They allow users to withdraw from Tornadocash to a fresh
//! address that has no ETH to pay for gas at the cost of a small fee. Relayers are run by
//! independent parties, and the Tornadocash DAO does not endorse any particular relayers.
//!
//! See [tornado-relayer](https://github.com/tornado-dao/tornado-relayer/tree/mainnet-v5) for
//! the reference implementation.
//!
//! # Example
//! ```no_run
//! # use alloy::{
//! #     primitives::{Address, U256},
//! #     providers::{DynProvider, Provider},
//! # };
//! # use kohaku_tornadocash::{
//! #     merkle_tree::{MerkleTree, MerkleTreeExt},
//! #     Note,
//! #     Pool,
//! #     Withdrawal,
//! # };
//! # use kohaku_tornadocash_relayer::Relayer;
//! #
//! # async fn example(
//! #     provider: DynProvider,
//! #     tree: &MerkleTree,
//! #     pool: Pool,
//! #     note: Note,
//! #     recipient: Address,
//! #     rng: &mut impl rand::CryptoRng,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let relayer = Relayer::new("https://mainnet.relayer.com");
//!
//! let gas_price = provider.get_gas_price().await?;
//! let payer = relayer.quote(&pool, gas_price, U256::ZERO).await?;
//! let merkle_proof = tree.note_proof(&note)?;
//!
//! let withdrawal = Withdrawal::new(&pool, note, recipient)
//!     .with_payer(payer)
//!     .prove(&merkle_proof, rng)?;
//!
//! // Confirmation is judged by the nullifier being spent on-chain, not by the relayer's report.
//! let receipt = relayer.withdraw(withdrawal).await?;
//! let tx_hash = relayer.await_confirmation(&provider, &receipt).await?;
//! println!("{tx_hash:?}");
//!
//! # Ok(())
//! # }
//! ```
use std::time::Duration;

use alloy::{
    primitives::{TxHash, U256},
    providers::Provider,
};
use kohaku_tornadocash::{Field, Payer, Pool, ProvenWithdrawal, TornadoProviderExt};
use serde::{Deserialize, Serialize};

mod status;
mod wire;

pub use status::{Health, Instance, RelayerStatus};
pub use wire::{JobId, JobStatus};

use crate::wire::{JobResponse, WithdrawRequest, WithdrawResponse};

/// Tornadocash relayer.
///
/// Interacts with a relayer client to create, submit, and monitor withdrawal proofs.
pub struct Relayer {
    url: String,
    client: reqwest::Client,

    timeout: Duration,
    poll_interval: Duration,
}

/// A receipt for a relayer job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobReceipt {
    pub id: JobId,
    pub pool: Pool,
    pub nullifier_hash: Field,
}

/// The status of a relayed withdrawal.
///
/// `spent` should be considered authoritative, because it's queried directly
/// from the chain. `tx_hash` and `job` are reported by the relayer which may be
/// unreliable or malicious.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawalStatus {
    /// Whether the nullifier has been spent on-chain.
    pub spent: bool,
    /// The reported transaction hash, if any.
    pub tx_hash: Option<TxHash>,
    /// The reported job status, if any.
    pub job: Option<JobStatus>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RelayerError {
    #[error("Unsupported pool: {0}")]
    UnsupportedPool(Pool),
    #[error("Reqwest error: {0}")]
    Client(#[from] reqwest::Error),
    #[error("Provider error: {0}")]
    Provider(#[from] alloy::contract::Error),
    #[error("Relayer error: {reason}")]
    JobFailed {
        reason: String,
        tx_hash: Option<TxHash>,
    },
    #[error("Timeout after {timeout:?} (last status: ({last_status:?}): {reason})")]
    Timeout {
        timeout: Duration,
        last_status: Option<JobStatus>,
        reason: String,
    },
}

impl Relayer {
    #[must_use]
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            client: reqwest::Client::new(),
            timeout: Duration::from_secs(30),
            poll_interval: Duration::from_millis(500),
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Retrieves the relayer's status.
    ///
    /// See <https://github.com/tornado-dao/tornado-relayer/blob/52473197ea49fb70dab8fead01de52545801ca6b/src/contollers/status.js#L7>
    /// for the reference implementation.
    pub async fn status(&self) -> Result<RelayerStatus, RelayerError> {
        let url = format!("{}/v1/status", self.url);
        let response: RelayerStatus = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response)
    }

    /// Quotes the fee for a withdrawal paid by the relayer.
    pub async fn quote(
        &self,
        pool: &Pool,
        gas_price: u128,
        refund: U256,
    ) -> Result<Payer, RelayerError> {
        let status = self.status().await?;
        status.quote(pool, gas_price, refund)
    }

    /// Submits a withdrawal job to the relayer.
    ///
    /// See <https://github.com/tornado-dao/tornado-relayer/blob/52473197ea49fb70dab8fead01de52545801ca6b/src/contollers/controller.js#L9>
    /// for the reference implementation.
    pub async fn withdraw(&self, withdrawal: ProvenWithdrawal) -> Result<JobReceipt, RelayerError> {
        let nullifier_hash = withdrawal.note().nullifier_hash();
        let request = WithdrawRequest {
            contract: withdrawal.pool().address,
            proof: withdrawal.solidity_proof(),
            args: (
                withdrawal.root().into(),
                nullifier_hash.into(),
                withdrawal.recipient(),
                withdrawal.payer().address,
                withdrawal.payer().fee.into(),
                withdrawal.payer().refund.into(),
            ),
        };

        let url = format!("{}/v1/tornadoWithdraw", self.url);
        let response: WithdrawResponse = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(JobReceipt {
            id: response.id,
            nullifier_hash,
            pool: withdrawal.pool().clone(),
        })
    }

    /// Checks the status of a relayed withdrawal.
    ///
    /// # Errors
    /// Returns an error if the relayer cannot be reached, the relayer returns an error, or the
    /// provider returns an error.
    pub async fn check(
        &self,
        provider: &impl Provider,
        receipt: &JobReceipt,
    ) -> Result<WithdrawalStatus, RelayerError> {
        let (job, spent) = tokio::join!(
            self.job_status(&receipt.id),
            provider.is_spent(&receipt.pool, receipt.nullifier_hash)
        );
        let spent = spent?;

        //? If the nullifier is spent, we don't care about the job status so early-exit.
        if spent {
            let job = job.ok();
            return Ok(WithdrawalStatus {
                spent: true,
                tx_hash: job.as_ref().and_then(|j| j.tx_hash),
                job: job.as_ref().map(|j| j.status),
            });
        }

        let job = job?;
        if job.status == JobStatus::Failed {
            return Err(RelayerError::JobFailed {
                reason: job.failed_reason.unwrap_or_else(|| "failed".to_string()),
                tx_hash: job.tx_hash,
            });
        }

        Ok(WithdrawalStatus {
            spent: false,
            tx_hash: job.tx_hash,
            job: Some(job.status),
        })
    }

    /// Polls until `receipt`'s withdrawal has landed on-chain.
    ///
    /// Returns the transaction hash or `None` if one was not supplied by the relayer.
    #[cfg(native)]
    pub async fn await_confirmation(
        &self,
        provider: &impl Provider,
        receipt: &JobReceipt,
    ) -> Result<Option<TxHash>, RelayerError> {
        let start = std::time::Instant::now();
        let mut last_status;

        loop {
            let status = self.check(provider, receipt).await?;
            if status.spent {
                return Ok(status.tx_hash);
            }

            last_status = status.job;

            if start.elapsed() > self.timeout {
                return Err(RelayerError::Timeout {
                    timeout: self.timeout,
                    last_status,
                    reason: "withdrawal did not land on-chain in time".to_string(),
                });
            }

            tokio::time::sleep(self.poll_interval).await;
        }
    }

    /// Polls the relayer for a withdrawal job's status.
    ///
    /// See <https://github.com/tornado-dao/tornado-relayer/blob/52473197ea49fb70dab8fead01de52545801ca6b/src/contollers/status.js#L32>
    /// for the reference implementation.
    async fn job_status(&self, id: &JobId) -> Result<JobResponse, RelayerError> {
        let url = format!("{}/v1/jobs/{}", self.url, id);
        let response: JobResponse = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response)
    }
}
