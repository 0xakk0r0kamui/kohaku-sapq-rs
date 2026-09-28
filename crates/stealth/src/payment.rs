use alloy::{
    primitives::{Address, U256},
    rpc::types::TransactionRequest,
};
use rand::CryptoRng;
use thiserror::Error;
use tracing::debug;

use crate::{
    contracts::{self, Asset, Deployment},
    scheme3::{Announcement, SchemeError, StealthMetaAddress, generate_stealth_address},
};

#[derive(Debug, Error)]
pub enum PaymentError {
    #[error(transparent)]
    Scheme(#[from] SchemeError),
    #[error("payment asset and amount are not configured")]
    MissingTransfer,
    #[error("payment amount must be non-zero")]
    ZeroAmount,
}

/// Unsigned transactions for one payment. Confirm the announcement succeeds before funding.
#[derive(Debug)]
pub struct PreparedPayment {
    pub stealth_address: Address,
    pub announcement: Announcement,
    pub announcement_transaction: TransactionRequest,
    pub funding_transaction: TransactionRequest,
}

#[derive(Debug)]
pub struct PaymentBuilder<'a> {
    deployment: Deployment,
    recipient: &'a StealthMetaAddress,
    transfer: Option<(Asset, U256)>,
}

impl<'a> PaymentBuilder<'a> {
    pub(crate) const fn new(deployment: Deployment, recipient: &'a StealthMetaAddress) -> Self {
        Self {
            deployment,
            recipient,
            transfer: None,
        }
    }

    #[must_use]
    pub fn native(mut self, amount: U256) -> Self {
        self.transfer = Some((Asset::Native, amount));
        self
    }

    #[must_use]
    pub fn erc20(mut self, token: Address, amount: U256) -> Self {
        self.transfer = Some((Asset::Erc20(token), amount));
        self
    }

    /// Generates fresh announcement material and prepares unsigned announcement and funding
    /// transactions.
    ///
    /// # Errors
    ///
    /// Returns an error when no transfer is configured, the amount is zero, or scheme 3
    /// generation fails.
    pub fn prepare(self, rng: &mut impl CryptoRng) -> Result<PreparedPayment, PaymentError> {
        let (asset, amount) = self.transfer.ok_or(PaymentError::MissingTransfer)?;
        if amount.is_zero() {
            return Err(PaymentError::ZeroAmount);
        }
        let generated = generate_stealth_address(self.recipient, rng)?;
        let announcement_transaction =
            contracts::prepare_announcement(&self.deployment, &generated.announcement);
        let funding_transaction =
            contracts::prepare_funding(&self.deployment, asset, generated.stealth_address, amount);
        debug!(
            chain_id = self.deployment.chain_id,
            stealth_address = %generated.stealth_address,
            ?asset,
            %amount,
            "prepared stealth payment"
        );
        Ok(PreparedPayment {
            stealth_address: generated.stealth_address,
            announcement: generated.announcement,
            announcement_transaction,
            funding_transaction,
        })
    }
}
