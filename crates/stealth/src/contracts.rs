use alloy::{
    network::TransactionBuilder,
    primitives::{Address, U256},
    rpc::types::TransactionRequest,
    sol_types::SolCall,
};
use pqsa_core::StealthScheme;
use pqsa_per_payment::SchemeId3;
use tracing::debug;

use crate::{
    abis::{IERC5564Announcer, IERC6538Registry},
    scheme3::{StealthAddress, StealthMetaAddress},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deployment {
    pub chain_id: u64,
    pub announcer: Address,
    pub registry: Address,
    pub start_block: u64,
}

/// Builds an unsigned ERC-6538 `registerKeys` transaction.
pub fn register_keys(
    deployment: &Deployment,
    meta_address: &StealthMetaAddress,
) -> TransactionRequest {
    let call = IERC6538Registry::registerKeysCall {
        schemeId: U256::from(SchemeId3::SCHEME_ID),
        stealthMetaAddress: SchemeId3::meta_to_bytes(meta_address).into(),
    };
    debug!(
        chain_id = deployment.chain_id,
        registry = %deployment.registry,
        meta_address_len = call.stealthMetaAddress.len(),
        "prepared stealth-key registration"
    );
    TransactionRequest::default()
        .with_chain_id(deployment.chain_id)
        .with_to(deployment.registry)
        .with_input(call.abi_encode())
}

/// Builds an unsigned ERC-6538 `registerKeysOnBehalf` transaction.
pub fn register_keys_on_behalf(
    deployment: &Deployment,
    registrant: Address,
    signature: &[u8],
    meta_address: &StealthMetaAddress,
) -> TransactionRequest {
    let call = IERC6538Registry::registerKeysOnBehalfCall {
        registrant,
        schemeId: U256::from(SchemeId3::SCHEME_ID),
        signature: signature.to_vec().into(),
        stealthMetaAddress: SchemeId3::meta_to_bytes(meta_address).into(),
    };
    debug!(
        chain_id = deployment.chain_id,
        registry = %deployment.registry,
        signature_len = signature.len(),
        meta_address_len = call.stealthMetaAddress.len(),
        "prepared delegated stealth-key registration"
    );
    TransactionRequest::default()
        .with_chain_id(deployment.chain_id)
        .with_to(deployment.registry)
        .with_input(call.abi_encode())
}

/// Builds an unsigned ERC-5564 announcement transaction.
pub fn prepare_announcement(
    deployment: &Deployment,
    stealth_address: &StealthAddress,
) -> TransactionRequest {
    let (address, ephemeral_public_key, metadata) =
        SchemeId3::announcement_to_bytes(stealth_address);
    let call = IERC5564Announcer::announceCall {
        schemeId: U256::from(SchemeId3::SCHEME_ID),
        stealthAddress: Address::from(address),
        ephemeralPubKey: ephemeral_public_key.into(),
        metadata: metadata.into(),
    };
    debug!(
        chain_id = deployment.chain_id,
        announcer = %deployment.announcer,
        ephemeral_public_key_len = call.ephemeralPubKey.len(),
        metadata_len = call.metadata.len(),
        "prepared stealth announcement"
    );
    TransactionRequest::default()
        .with_chain_id(deployment.chain_id)
        .with_to(deployment.announcer)
        .with_input(call.abi_encode())
}
