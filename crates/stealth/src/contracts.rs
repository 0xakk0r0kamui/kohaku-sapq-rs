use alloy::{
    network::TransactionBuilder,
    primitives::{Address, U256},
    providers::{DynProvider, Provider},
    rpc::types::TransactionRequest,
    sol,
    sol_types::SolCall,
};
use thiserror::Error;
use tracing::debug;

use crate::scheme3::{Announcement, SCHEME_ID, SchemeError, StealthMetaAddress};

sol! {
    interface IERC5564Announcer {
        event Announcement(
            uint256 indexed schemeId,
            address indexed stealthAddress,
            address indexed caller,
            bytes ephemeralPubKey,
            bytes metadata
        );
        function announce(
            uint256 schemeId,
            address stealthAddress,
            bytes calldata ephemeralPubKey,
            bytes calldata metadata
        ) external;
    }

    interface IERC6538Registry {
        function registerKeys(uint256 schemeId, bytes calldata stealthMetaAddress) external;
        function registerKeysOnBehalf(
            address registrant,
            uint256 schemeId,
            bytes calldata signature,
            bytes calldata stealthMetaAddress
        ) external;
        function stealthMetaAddressOf(address registrant, uint256 schemeId)
            external
            view
            returns (bytes memory stealthMetaAddress);
    }

    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deployment {
    pub chain_id: u64,
    pub announcer: Address,
    pub registry: Address,
    pub start_block: u64,
}

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("contract RPC call failed")]
    Rpc(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("RPC is connected to chain {actual}; deployment expects chain {expected}")]
    WrongChain { expected: u64, actual: u64 },
    #[error("registry returned an invalid scheme 3 meta-address")]
    InvalidMetaAddress(#[source] SchemeError),
}

fn rpc(error: impl std::error::Error + Send + Sync + 'static) -> ContractError {
    ContractError::Rpc(Box::new(error))
}

#[must_use]
pub fn prepare_registration(
    deployment: &Deployment,
    meta_address: &StealthMetaAddress,
) -> TransactionRequest {
    let call = IERC6538Registry::registerKeysCall {
        schemeId: U256::from(SCHEME_ID),
        stealthMetaAddress: meta_address.as_bytes().to_vec().into(),
    };
    debug!(
        chain_id = deployment.chain_id,
        registry = %deployment.registry,
        meta_address_len = meta_address.as_bytes().len(),
        "prepared stealth-key registration"
    );
    TransactionRequest::default()
        .with_chain_id(deployment.chain_id)
        .with_to(deployment.registry)
        .with_input(call.abi_encode())
}

#[must_use]
pub fn prepare_registration_on_behalf(
    deployment: &Deployment,
    registrant: Address,
    signature: &[u8],
    meta_address: &StealthMetaAddress,
) -> TransactionRequest {
    let call = IERC6538Registry::registerKeysOnBehalfCall {
        registrant,
        schemeId: U256::from(SCHEME_ID),
        signature: signature.to_vec().into(),
        stealthMetaAddress: meta_address.as_bytes().to_vec().into(),
    };
    debug!(
        chain_id = deployment.chain_id,
        registry = %deployment.registry,
        %registrant,
        signature_len = signature.len(),
        meta_address_len = meta_address.as_bytes().len(),
        "prepared delegated stealth-key registration"
    );
    TransactionRequest::default()
        .with_chain_id(deployment.chain_id)
        .with_to(deployment.registry)
        .with_input(call.abi_encode())
}

#[must_use]
pub fn prepare_announcement(
    deployment: &Deployment,
    announcement: &Announcement,
) -> TransactionRequest {
    let call = IERC5564Announcer::announceCall {
        schemeId: U256::from(SCHEME_ID),
        stealthAddress: announcement.stealth_address(),
        ephemeralPubKey: announcement.ephemeral_public_key().to_vec().into(),
        metadata: announcement.metadata().to_vec().into(),
    };
    debug!(
        chain_id = deployment.chain_id,
        announcer = %deployment.announcer,
        stealth_address = %announcement.stealth_address(),
        ephemeral_public_key_len = announcement.ephemeral_public_key().len(),
        metadata_len = announcement.metadata().len(),
        "prepared stealth announcement"
    );
    TransactionRequest::default()
        .with_chain_id(deployment.chain_id)
        .with_to(deployment.announcer)
        .with_input(call.abi_encode())
}

/// Reads and validates a registrant's scheme 3 meta-address.
///
/// # Errors
///
/// Returns [`ContractError::Rpc`] when the call fails,
/// [`ContractError::WrongChain`] when the RPC chain does not match the deployment, or
/// [`ContractError::InvalidMetaAddress`] when the registry value is not valid for scheme 3.
pub async fn resolve_meta_address(
    provider: &DynProvider,
    deployment: &Deployment,
    registrant: Address,
) -> Result<StealthMetaAddress, ContractError> {
    let actual_chain_id = provider.get_chain_id().await.map_err(rpc)?;
    if actual_chain_id != deployment.chain_id {
        return Err(ContractError::WrongChain {
            expected: deployment.chain_id,
            actual: actual_chain_id,
        });
    }
    let call = IERC6538Registry::stealthMetaAddressOfCall {
        registrant,
        schemeId: U256::from(SCHEME_ID),
    };
    let output = provider
        .call(
            TransactionRequest::default()
                .with_chain_id(deployment.chain_id)
                .with_to(deployment.registry)
                .with_input(call.abi_encode()),
        )
        .await
        .map_err(rpc)?;
    let bytes =
        IERC6538Registry::stealthMetaAddressOfCall::abi_decode_returns(&output).map_err(rpc)?;
    let meta_address =
        StealthMetaAddress::from_bytes(bytes).map_err(ContractError::InvalidMetaAddress)?;
    debug!(
        chain_id = deployment.chain_id,
        %registrant,
        meta_address_len = meta_address.as_bytes().len(),
        "resolved stealth meta-address"
    );
    Ok(meta_address)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Asset {
    Native,
    Erc20(Address),
}

#[must_use]
pub(crate) fn prepare_funding(
    deployment: &Deployment,
    asset: Asset,
    recipient: Address,
    amount: U256,
) -> TransactionRequest {
    match asset {
        Asset::Native => TransactionRequest::default()
            .with_chain_id(deployment.chain_id)
            .with_to(recipient)
            .with_value(amount),
        Asset::Erc20(token) => {
            let call = IERC20::transferCall {
                to: recipient,
                amount,
            };
            TransactionRequest::default()
                .with_chain_id(deployment.chain_id)
                .with_to(token)
                .with_input(call.abi_encode())
        }
    }
}

pub(crate) use IERC5564Announcer::Announcement as AnnouncementEvent;
