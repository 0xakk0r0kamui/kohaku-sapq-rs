use alloy::{primitives::Address, providers::Provider};
use ruint::aliases::U256;

use crate::{abis::tornado::Tornado, asset::Asset, field::Field, pool::Pool};

/// Extension trait for [`Provider`] with Tornadocash-specific accessors.
pub trait TornadoProviderExt: Provider {
    /// Indicates whether `nullifier_hash` has been spent.
    fn is_spent(
        &self,
        pool: &Pool,
        nullifier_hash: Field,
    ) -> impl Future<Output = Result<bool, alloy::contract::Error>>;

    /// Indicates whether each of `nullifier_hashes` has been spent.
    ///
    /// The returned vector will have the same length and ordering as `nullifier_hashes`.
    fn is_spent_array(
        &self,
        pool: &Pool,
        nullifier_hashes: &[Field],
    ) -> impl Future<Output = Result<Vec<bool>, alloy::contract::Error>>;

    /// Indicates whether `root` is known to the Tornado pool.
    fn is_known_root(
        &self,
        pool: &Pool,
        root: Field,
    ) -> impl Future<Output = Result<bool, alloy::contract::Error>>;

    /// Returns the amount of fee token equivalent to the given `wei_amount`.
    ///
    /// If the pool's asset is native, returns `wei_amount` directly.
    fn quote_wei_in_fee_token(
        &self,
        pool: &Pool,
        wei_amount: U256,
    ) -> impl Future<Output = Result<U256, alloy::contract::Error>>;

    /// Returns the address of the contract that verifies the pool's withdrawal proofs.
    fn verifier(
        &self,
        pool: &Pool,
    ) -> impl Future<Output = Result<Address, alloy::contract::Error>>;
}

impl<P: Provider> TornadoProviderExt for P {
    async fn is_spent(
        &self,
        pool: &Pool,
        nullifier_hash: Field,
    ) -> Result<bool, alloy::contract::Error> {
        Tornado::new(pool.address, self)
            .isSpent(nullifier_hash.into())
            .call()
            .await
    }

    async fn is_spent_array(
        &self,
        pool: &Pool,
        nullifier_hashes: &[Field],
    ) -> Result<Vec<bool>, alloy::contract::Error> {
        let hashes: Vec<_> = nullifier_hashes.iter().map(|&h| h.into()).collect();
        Tornado::new(pool.address, self)
            .isSpentArray(hashes)
            .call()
            .await
    }

    async fn is_known_root(
        &self,
        pool: &Pool,
        root: Field,
    ) -> Result<bool, alloy::contract::Error> {
        Tornado::new(pool.address, self)
            .isKnownRoot(root.into())
            .call()
            .await
    }

    async fn quote_wei_in_fee_token(
        &self,
        pool: &Pool,
        wei_amount: U256,
    ) -> Result<U256, alloy::contract::Error> {
        match pool.asset {
            Asset::Native { .. } => Ok(wei_amount),
            Asset::Erc20 { address, .. } => {
                Tornado::new(pool.address, self)
                    .quoteWeiInToken(address, wei_amount)
                    .call()
                    .await
            }
        }
    }

    async fn verifier(&self, pool: &Pool) -> Result<Address, alloy::contract::Error> {
        Tornado::new(pool.address, self).verifier().call().await
    }
}
