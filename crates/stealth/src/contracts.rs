use alloy::{
    network::TransactionBuilder,
    primitives::{Address, U256},
    providers::Provider,
    rpc::types::TransactionRequest,
    sol,
    sol_types::SolCall,
    transports::TransportError,
};
use pqsa_core::StealthScheme;
use pqsa_kem::{Kem, MlKem768};
use pqsa_per_payment::SchemeId3;
use thiserror::Error;
use tracing::debug;

use crate::scheme3::{EngineError, SCHEME_ID, SchemeError, StealthAddress, StealthMetaAddress};

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
            bytes memory ephemeralPubKey,
            bytes memory metadata
        ) external;
    }

    #[sol(rpc)]
    interface IERC6538Registry {
        function registerKeys(uint256 schemeId, bytes calldata stealthMetaAddress) external;
        function registerKeysOnBehalf(
            address registrant,
            uint256 schemeId,
            bytes memory signature,
            bytes calldata stealthMetaAddress
        ) external;
        function stealthMetaAddressOf(address registrant, uint256 schemeId)
            external
            view
            returns (bytes memory stealthMetaAddress);
        function nonceOf(address registrant) external view returns (uint256);
        function DOMAIN_SEPARATOR() external view returns (bytes32);
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
    #[error(transparent)]
    Rpc(#[from] TransportError),
    #[error("registry returned an invalid ABI response")]
    InvalidResponse(#[source] alloy::sol_types::Error),
    #[error("RPC is connected to chain {actual}; deployment expects chain {expected}")]
    WrongChain { expected: u64, actual: u64 },
    #[error("invalid scheme 3 meta-address")]
    InvalidMetaAddress(#[source] SchemeError),
    #[error("invalid scheme 3 announcement")]
    InvalidAnnouncement(#[source] SchemeError),
}

pub fn prepare_registration(
    deployment: &Deployment,
    meta_address: &StealthMetaAddress,
) -> Result<TransactionRequest, ContractError> {
    let call = IERC6538Registry::registerKeysCall {
        schemeId: U256::from(SCHEME_ID),
        stealthMetaAddress: meta_address_bytes(meta_address)?.into(),
    };
    debug!(
        chain_id = deployment.chain_id,
        registry = %deployment.registry,
        meta_address_len = call.stealthMetaAddress.len(),
        "prepared stealth-key registration"
    );
    Ok(TransactionRequest::default()
        .with_chain_id(deployment.chain_id)
        .with_to(deployment.registry)
        .with_input(call.abi_encode()))
}

pub fn prepare_registration_on_behalf(
    deployment: &Deployment,
    registrant: Address,
    signature: &[u8],
    meta_address: &StealthMetaAddress,
) -> Result<TransactionRequest, ContractError> {
    let call = IERC6538Registry::registerKeysOnBehalfCall {
        registrant,
        schemeId: U256::from(SCHEME_ID),
        signature: signature.to_vec().into(),
        stealthMetaAddress: meta_address_bytes(meta_address)?.into(),
    };
    debug!(
        chain_id = deployment.chain_id,
        registry = %deployment.registry,
        %registrant,
        signature_len = signature.len(),
        meta_address_len = call.stealthMetaAddress.len(),
        "prepared delegated stealth-key registration"
    );
    Ok(TransactionRequest::default()
        .with_chain_id(deployment.chain_id)
        .with_to(deployment.registry)
        .with_input(call.abi_encode()))
}

fn meta_address_bytes(meta_address: &StealthMetaAddress) -> Result<Vec<u8>, ContractError> {
    if meta_address.viewing_ec.is_none() || meta_address.ek.len() != MlKem768::EK_BYTES {
        return Err(ContractError::InvalidMetaAddress(
            EngineError::Malformed.into(),
        ));
    }
    Ok(SchemeId3::meta_to_bytes(meta_address))
}

/// Builds an unsigned ERC-5564 announcement transaction.
/// Fails if the ephemeral point is missing or the ciphertext has the wrong length.
pub fn prepare_announcement(
    deployment: &Deployment,
    stealth_address: &StealthAddress,
) -> Result<TransactionRequest, ContractError> {
    if stealth_address.epk.is_none() || stealth_address.ct.len() != MlKem768::CT_BYTES {
        return Err(ContractError::InvalidAnnouncement(
            EngineError::Malformed.into(),
        ));
    }
    let (address, ephemeral_public_key, metadata) =
        SchemeId3::announcement_to_bytes(stealth_address);
    let call = IERC5564Announcer::announceCall {
        schemeId: U256::from(SCHEME_ID),
        stealthAddress: Address::from(address),
        ephemeralPubKey: ephemeral_public_key.into(),
        metadata: metadata.into(),
    };
    debug!(
        chain_id = deployment.chain_id,
        announcer = %deployment.announcer,
        stealth_address = %Address::from(address),
        ephemeral_public_key_len = call.ephemeralPubKey.len(),
        metadata_len = call.metadata.len(),
        "prepared stealth announcement"
    );
    Ok(TransactionRequest::default()
        .with_chain_id(deployment.chain_id)
        .with_to(deployment.announcer)
        .with_input(call.abi_encode()))
}

/// Looks up a scheme 3 meta-address in the registry.
/// Returns `None` if the registry entry is empty.
///
/// # Errors
///
/// Fails on a wrong chain, an RPC error, or an invalid registry response.
pub async fn resolve_meta_address(
    provider: &impl Provider,
    deployment: &Deployment,
    registrant: Address,
) -> Result<Option<StealthMetaAddress>, ContractError> {
    let actual_chain_id = provider.get_chain_id().await?;
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
        .await?;
    let bytes = IERC6538Registry::stealthMetaAddressOfCall::abi_decode_returns(&output)
        .map_err(ContractError::InvalidResponse)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let meta_address = SchemeId3::meta_from_bytes(&bytes)
        .ok_or_else(|| ContractError::InvalidMetaAddress(EngineError::Malformed.into()))?;
    debug!(
        chain_id = deployment.chain_id,
        %registrant,
        meta_address_len = bytes.len(),
        "resolved stealth meta-address"
    );
    Ok(Some(meta_address))
}

pub(crate) use IERC5564Announcer::Announcement as AnnouncementEvent;

#[cfg(test)]
mod tests {
    use alloy::{
        primitives::{Bytes, TxKind},
        providers::ProviderBuilder,
        transports::mock::Asserter,
    };

    use super::*;
    use crate::Scheme3Account;

    const DEPLOYMENT: Deployment = Deployment {
        chain_id: 1,
        announcer: Address::repeat_byte(0x11),
        registry: Address::repeat_byte(0x22),
        start_block: 0,
    };

    #[test]
    fn announcement_transaction() {
        let mut metadata = vec![0x55; 1089];
        metadata[0] = 0x44;
        let announcement =
            SchemeId3::announcement_from_bytes(&[0x33; 20], &[0x02; 33], &metadata).unwrap();
        let transaction = prepare_announcement(&DEPLOYMENT, &announcement).unwrap();

        assert_eq!(transaction.to, Some(TxKind::Call(DEPLOYMENT.announcer)));
        assert_eq!(transaction.chain_id, Some(DEPLOYMENT.chain_id));
        let call = IERC5564Announcer::announceCall::abi_decode(transaction.input.input().unwrap())
            .unwrap();
        assert_eq!(call.schemeId, U256::from(SCHEME_ID));
        assert_eq!(
            call.stealthAddress,
            Address::from(announcement.stealth_address)
        );
        assert_eq!(
            call.ephemeralPubKey.as_ref(),
            announcement.epk.as_ref().unwrap().as_bytes()
        );
        assert_eq!(call.metadata.len(), 1089);
        assert_eq!(call.metadata[0], announcement.view_tag[0]);
        assert_eq!(&call.metadata[1..], announcement.ct);
    }

    #[test]
    fn invalid_meta_address() {
        let account = Scheme3Account::from_keygen_master(&[7; 32])
            .unwrap()
            .account;
        let meta = account.meta_address();
        for (viewing, len) in [
            (None, MlKem768::EK_BYTES),
            (meta.viewing_ec, MlKem768::EK_BYTES - 1),
            (meta.viewing_ec, MlKem768::EK_BYTES + 1),
        ] {
            let mut malformed = meta.clone();
            malformed.viewing_ec = viewing;
            malformed.ek.resize(len, 0);
            for result in [
                prepare_registration(&DEPLOYMENT, &malformed),
                prepare_registration_on_behalf(&DEPLOYMENT, Address::ZERO, &[0x77; 65], &malformed),
            ] {
                assert!(matches!(
                    result,
                    Err(ContractError::InvalidMetaAddress(SchemeError::Engine(
                        EngineError::Malformed
                    )))
                ));
            }
        }
    }

    #[test]
    fn invalid_announcement() {
        let account = Scheme3Account::from_keygen_master(&[7; 32])
            .unwrap()
            .account;
        let announcement = SchemeId3::announce(account.meta_address(), &[0x44; 64]).unwrap();
        for (epk, len) in [
            (None, MlKem768::CT_BYTES),
            (announcement.epk, MlKem768::CT_BYTES - 1),
            (announcement.epk, MlKem768::CT_BYTES + 1),
        ] {
            let mut malformed = announcement.clone();
            malformed.epk = epk;
            malformed.ct.resize(len, 0);
            assert!(matches!(
                prepare_announcement(&DEPLOYMENT, &malformed),
                Err(ContractError::InvalidAnnouncement(SchemeError::Engine(
                    EngineError::Malformed
                )))
            ));
        }
    }

    #[tokio::test]
    async fn empty_registry_entry() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&"0x1");
        let encoded = IERC6538Registry::stealthMetaAddressOfCall::abi_encode_returns(&Bytes::new());
        asserter.push_success(&Bytes::from(encoded));

        assert!(
            resolve_meta_address(&provider, &DEPLOYMENT, Address::ZERO)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn invalid_registry_response() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        for output in [Bytes::new(), Bytes::from_static(&[0x01])] {
            asserter.push_success(&"0x1");
            asserter.push_success(&output);
            let error = resolve_meta_address(&provider, &DEPLOYMENT, Address::ZERO)
                .await
                .unwrap_err();
            assert!(matches!(error, ContractError::InvalidResponse(_)));
            assert!(
                std::error::Error::source(&error)
                    .unwrap()
                    .is::<alloy::sol_types::Error>()
            );
        }

        for bytes in [Bytes::from(vec![0; 1_249]), vec![0; 1_250].into()] {
            asserter.push_success(&"0x1");
            let encoded = IERC6538Registry::stealthMetaAddressOfCall::abi_encode_returns(&bytes);
            asserter.push_success(&Bytes::from(encoded));
            let error = resolve_meta_address(&provider, &DEPLOYMENT, Address::ZERO)
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                ContractError::InvalidMetaAddress(SchemeError::Engine(EngineError::Malformed))
            ));
        }
    }
}
