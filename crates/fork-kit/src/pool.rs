//! Tornadocash pool deployment.

use alloy::{
    primitives::{Address, U256},
    providers::{DynProvider, Provider},
};
use kohaku_tornadocash::{Asset, Pool};

mod sol {
    #![allow(clippy::all)]
    #![allow(clippy::pedantic)]

    use alloy::sol;

    sol!(
        #[sol(rpc)]
        Hasher,
        "fixtures/hasher.json"
    );
    sol!(
        #[sol(rpc)]
        Verifier,
        "fixtures/verifier.json"
    );
    sol!(
        #[sol(rpc)]
        ETHTornado,
        "fixtures/eth_tornado.json"
    );
    sol!(
        #[sol(rpc)]
        TornadoProxyLight,
        "fixtures/tornado_proxy_light.json"
    );
}

const DEFAULT_DENOMINATION_WEI: u128 = 10_u128.pow(17);
const MERKLE_TREE_HEIGHT: u32 = 20;

/// Deploys a fresh tornadocash pool contract.
///
/// Returns the deployed [`Pool`] with a denomination `denomination_wei` if provided.
///
/// # Errors
/// Returns an error if any contract fails to deploy.
pub async fn deploy_pool(
    provider: DynProvider,
    denomination_wei: Option<u128>,
) -> Result<Pool, anyhow::Error> {
    let denomination_wei = denomination_wei.unwrap_or(DEFAULT_DENOMINATION_WEI);

    let hasher = sol::Hasher::deploy(&provider).await?;
    let verifier = sol::Verifier::deploy(&provider).await?;
    let tornado = sol::ETHTornado::deploy(
        &provider,
        *verifier.address(),
        *hasher.address(),
        U256::from(denomination_wei),
        MERKLE_TREE_HEIGHT,
    )
    .await?;

    Ok(Pool {
        chain_id: provider.get_chain_id().await?,
        address: *tornado.address(),
        asset: Asset::ETH,
        amount_wei: denomination_wei,
        deployed_block: 0,
        paymaster: None,
        canonical: false,
    })
}

/// Deploy a fresh `TornadoProxyLight` to `provider` and return its address.
///
/// # Errors
/// Returns an error if the contract fails to deploy.
pub async fn deploy_proxy(provider: DynProvider) -> Result<Address, anyhow::Error> {
    let proxy = sol::TornadoProxyLight::deploy(&provider).await?;
    Ok(*proxy.address())
}
