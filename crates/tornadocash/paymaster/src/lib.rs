//! Extension trait for building userops with a Tornadocash paymaster.
//!
//! # Example
//! ```no_run
//! # use alloy::{providers::DynProvider, signers::local::PrivateKeySigner};
//! # use kohaku_tornadocash::{
//! #     merkle_tree::{MerkleTree, MerkleTreeExt},
//! #     Note,
//! #     Pool,
//! #     Withdrawal,
//! # };
//! # use kohaku_tornadocash_paymaster::UserOperationPaymasterExt;
//! # use kohaku_userop_kit::{
//! #     builder::UserOperationBuilder,
//! #     bundler::{Bundler, pimlico::PimlicoBundler},
//! #     smart_account::simple_7702_smart_account::{Call, Simple7702SmartAccount},
//! # };
//! #
//! # async fn example(
//! #     provider: DynProvider,
//! #     tree: &MerkleTree,
//! #     pool: Pool,
//! #     note: Note,
//! #     rng: &mut impl rand::CryptoRng,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let owner = PrivateKeySigner::random();
//! let bundler = PimlicoBundler::new("https://bundler.pimlico.com".parse()?);
//!
//! let smart_account = Simple7702SmartAccount::new(provider.clone(), owner.address(), pool.chain_id);
//! let builder = UserOperationBuilder::new_with_smart_account(&smart_account)
//!     .await?
//!     .with_call(&vec![Call::default()]);
//!
//! // The note pays for gas, and its remainder is withdrawn to `owner` during validation, so the
//! // operation's own calls can spend it.
//! let withdrawal_merkle_proof = tree.note_proof(&note)?;
//! let withdrawal = Withdrawal::new(&pool, note, owner.address());
//! let builder = builder
//!     .with_tornado_paymaster(withdrawal, &withdrawal_merkle_proof, &provider, &bundler, rng)
//!     .await?;
//!
//! let userop = builder.build().sign(&owner).await?;
//! let hash = bundler.send_user_operation(&userop).await?;
//! println!("{hash:?}");
//!
//! Ok(())
//! # }
//! ```

use alloy::{
    primitives::{Address, Bytes, U256, address, hex},
    providers::Provider,
    rpc::types::state::{StateOverride, StateOverridesBuilder},
    sol_types::SolValue,
};
use kohaku_tornadocash::{
    Payer, PaymasterInfo, Pool, ProvenWithdrawal, TornadoProviderExt, Withdrawal, WithdrawalError,
    merkle_tree::MerkleProof,
};
use kohaku_userop_kit::{
    builder::UserOperationBuilder,
    bundler::{Bundler, BundlerExt},
};

use crate::abis::{PaymasterData, TornadoAdapterData};

mod abis;

/// Safety margin added to the quoted fee, in basis points.
const FEE_BUFFER_BPS: u128 = 100;

/// Fraction of the denomination offered as the fee for the gas estimate.
const SEED_FEE_DENOMINATOR: u64 = 10;

/// Arbitrary mocked address a pool's verifier is moved to during gas estimation.
const SHADOW_VERIFIER: Address = address!("0x00000000000000000000000000004b6f68616b75");

/// Extension trait for [`UserOperationBuilder`] that pays for an operation with a tornadocash
/// note.
///
/// Tornadocash paymasters use a shielded note to pay for a `UserOp`'s gas. The remainder of the
/// note is withdrawn to the recipient during validation and available during the operation's
/// execution.
pub trait UserOperationPaymasterExt: Sized {
    /// Pays for this operation's gas out of `withdrawal`'s note.
    ///
    /// # Errors
    /// Returns an error if the pool has no paymaster, if the note cannot cover the operation's
    /// gas, or if proving, estimation, or the fee quote fails.
    fn with_tornado_paymaster<R, B>(
        self,
        withdrawal: Withdrawal,
        merkle_proof: &MerkleProof,
        provider: &impl Provider,
        bundler: &B,
        rng: &mut R,
    ) -> impl std::future::Future<Output = Result<Self, TornadoPaymasterError>>
    where
        R: rand::CryptoRng,
        B: Bundler + ?Sized;
}

#[derive(Debug, thiserror::Error)]
pub enum TornadoPaymasterError {
    #[error("Pool missing paymaster: {0}")]
    PoolMissingPaymaster(Pool),
    #[error("Pool denomination cannot cover the operation's gas: {required} > {denomination}")]
    InsufficientDenomination { required: U256, denomination: U256 },
    #[error("Bundler error: {0}")]
    Bundler(#[from] kohaku_userop_kit::bundler::BundlerError),
    #[error("Provider error: {0}")]
    Provider(#[from] alloy::contract::Error),
    #[error("Transport error: {0}")]
    Transport(#[from] alloy::transports::TransportError),
    #[error("Withdrawal error: {0}")]
    Withdrawal(#[from] WithdrawalError),
    #[error("Merkle tree error: {0}")]
    MerkleTree(#[from] kohaku_merkle_tree::MerkleTreeError),
}

impl<S> UserOperationPaymasterExt for UserOperationBuilder<S> {
    #[tracing::instrument(skip_all)]
    async fn with_tornado_paymaster<R, B>(
        self,
        withdrawal: Withdrawal,
        merkle_proof: &MerkleProof,
        provider: &impl Provider,
        bundler: &B,
        rng: &mut R,
    ) -> Result<Self, TornadoPaymasterError>
    where
        R: rand::CryptoRng,
        B: Bundler + ?Sized,
    {
        let pool = &withdrawal.pool;
        let info = pool
            .paymaster
            .ok_or_else(|| TornadoPaymasterError::PoolMissingPaymaster(pool.clone()))?;

        let (builder, fee) =
            estimate_fee(self, &withdrawal, info, merkle_proof, provider, bundler).await?;

        //? The placeholder costs what a real proof does, so the estimate holds once proven.
        let proven = withdrawal
            .with_payer(Payer {
                address: info.address,
                fee,
                refund: U256::ZERO,
            })
            .prove(merkle_proof, rng)?;

        Ok(builder
            .with_paymaster_and_data(info.address, encode_paymaster_data(info.adapter, &proven)))
    }
}

/// Estimates the operation's gas.
async fn estimate_fee<S, B>(
    builder: UserOperationBuilder<S>,
    withdrawal: &Withdrawal,
    info: PaymasterInfo,
    merkle_proof: &MerkleProof,
    provider: &impl Provider,
    bundler: &B,
) -> Result<(UserOperationBuilder<S>, U256), TornadoPaymasterError>
where
    B: Bundler + ?Sized,
{
    let pool = &withdrawal.pool;
    let denomination = U256::from(pool.amount_wei);

    //? The paymaster still checks the fee against the bundler's max cost, and
    //? the operation's calls may spend what the fee leaves, so the seed fee must
    //? sit between the two.
    // TODO: Find a better way to handle this. Maybe override the paymaster to accept any fee?
    let seed = denomination / U256::from(SEED_FEE_DENOMINATOR);
    let placeholder = withdrawal
        .clone()
        .with_payer(Payer {
            address: info.address,
            fee: seed,
            refund: U256::ZERO,
        })
        .prove_dummy(merkle_proof);

    let overrides = verifier_override(provider, pool).await?;
    let builder = builder
        .with_paymaster_and_data(
            info.address,
            encode_paymaster_data(info.adapter, &placeholder),
        )
        .with_gas_estimate(bundler, Some(&overrides))
        .await?;

    let cost = gas_cost(provider, pool, &builder).await?;
    let fee = cost + cost * U256::from(FEE_BUFFER_BPS) / U256::from(10_000);
    if fee > denomination {
        return Err(TornadoPaymasterError::InsufficientDenomination {
            required: fee,
            denomination,
        });
    }

    Ok((builder, fee))
}

/// Overrides the pool's verifier contract to accept any proof while consuming similar gas.
///
/// The forwarder's extra call makes a withdrawal about 3k gas more expensive than with a real
/// proof, so estimates on err slightly high.
async fn verifier_override(
    provider: &impl Provider,
    pool: &Pool,
) -> Result<StateOverride, TornadoPaymasterError> {
    let verifier = provider.verifier(pool).await?;
    let code = provider.get_code_at(verifier).await?;

    Ok(StateOverridesBuilder::default()
        .with_code(SHADOW_VERIFIER, code)
        .with_code(verifier, forwarder(SHADOW_VERIFIER))
        .build())
}

/// Quotes the operation's maximum gas cost in the pool's fee token.
async fn gas_cost<S>(
    provider: &impl Provider,
    pool: &Pool,
    builder: &UserOperationBuilder<S>,
) -> Result<U256, TornadoPaymasterError> {
    let user_op = builder.build();
    let gas = U256::from(user_op.total_gas_limit());
    let price =
        U256::from(user_op.user_op.max_fee_per_gas + user_op.user_op.max_priority_fee_per_gas);

    Ok(provider.quote_wei_in_fee_token(pool, gas * price).await?)
}

fn encode_paymaster_data(adapter: Address, withdrawal: &ProvenWithdrawal) -> Bytes {
    let adapter_data = TornadoAdapterData {
        proof: withdrawal.solidity_proof(),
        root: withdrawal.root().into(),
        nullifierHash: withdrawal.note().nullifier_hash().into(),
        recipient: withdrawal.recipient(),
        relayer: withdrawal.payer().address,
        fee: withdrawal.payer().fee,
        refund: withdrawal.payer().refund,
    };
    let data = PaymasterData {
        adapter,
        adapterData: adapter_data.abi_encode().into(),
    };

    data.abi_encode().into()
}

/// Execution code that calls `target`, discards the outcome, and returns `true`.
fn forwarder(target: Address) -> Bytes {
    [
        // calldatacopy(0, 0, calldatasize)
        // staticcall(gas, target, 0, calldatasize, 0, 0)
        &hex!("3660006000376000600036600073")[..],
        target.as_slice(),
        // pop the call's success; mstore(0, 1); return(0, 32)
        &hex!("5afa50600160005260206000f3"),
    ]
    .concat()
    .into()
}
