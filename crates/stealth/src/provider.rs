use alloy::{
    primitives::{Address, U256},
    providers::Provider,
    transports::TransportError,
};
use pqsa_core::StealthScheme;
use pqsa_per_payment::SchemeId3;
use thiserror::Error;
use tracing::debug;

use crate::{abis::IERC6538Registry, contracts::Deployment, scheme3::StealthMetaAddress};

#[derive(Debug, Error)]
pub enum ContractError {
    #[error(transparent)]
    Rpc(#[from] TransportError),
    #[error(transparent)]
    Contract(#[from] alloy::contract::Error),
    #[error("RPC is connected to chain {actual}; deployment expects chain {expected}")]
    WrongChain { expected: u64, actual: u64 },
    #[error("invalid scheme 3 meta-address")]
    InvalidMetaAddress,
}

/// Stealth registry queries for Alloy providers.
pub trait StealthProviderExt: Provider {
    /// Looks up a scheme 3 meta-address. Returns `None` if the registry entry is empty.
    ///
    /// # Errors
    ///
    /// Fails on a wrong chain, an RPC or contract error, or an invalid meta-address.
    fn resolve_meta_address(
        &self,
        deployment: &Deployment,
        registrant: Address,
    ) -> impl Future<Output = Result<Option<StealthMetaAddress>, ContractError>>;
}

impl<P: Provider> StealthProviderExt for P {
    async fn resolve_meta_address(
        &self,
        deployment: &Deployment,
        registrant: Address,
    ) -> Result<Option<StealthMetaAddress>, ContractError> {
        let actual_chain_id = self.get_chain_id().await?;
        if actual_chain_id != deployment.chain_id {
            return Err(ContractError::WrongChain {
                expected: deployment.chain_id,
                actual: actual_chain_id,
            });
        }
        let bytes = IERC6538Registry::new(deployment.registry, self)
            .stealthMetaAddressOf(registrant, U256::from(SchemeId3::SCHEME_ID))
            .chain_id(deployment.chain_id)
            .call()
            .await?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let meta_address =
            SchemeId3::meta_from_bytes(&bytes).ok_or(ContractError::InvalidMetaAddress)?;
        debug!(
            chain_id = deployment.chain_id,
            meta_address_len = bytes.len(),
            "resolved stealth meta-address"
        );
        Ok(Some(meta_address))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use alloy::{
        primitives::Bytes, providers::ProviderBuilder, sol_types::SolCall,
        transports::mock::Asserter,
    };

    use super::*;

    const DEPLOYMENT: Deployment = Deployment {
        chain_id: 1,
        announcer: Address::repeat_byte(0x11),
        registry: Address::repeat_byte(0x22),
        start_block: 0,
    };

    #[tokio::test]
    async fn wrong_chain() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&"0x2");
        let encoded = IERC6538Registry::stealthMetaAddressOfCall::abi_encode_returns(&Bytes::new());
        asserter.push_success(&Bytes::from(encoded));

        let error = provider
            .resolve_meta_address(&DEPLOYMENT, Address::ZERO)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ContractError::WrongChain {
                expected: 1,
                actual: 2
            }
        ));
        assert_eq!(asserter.read_q().len(), 1, "registry lookup must not run");
    }

    #[tokio::test]
    async fn empty_registry_entry() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&"0x1");
        let encoded = IERC6538Registry::stealthMetaAddressOfCall::abi_encode_returns(&Bytes::new());
        asserter.push_success(&Bytes::from(encoded));

        assert!(
            provider
                .resolve_meta_address(&DEPLOYMENT, Address::ZERO)
                .await
                .unwrap()
                .is_none()
        );
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn invalid_registry_response() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        for output in [Bytes::new(), Bytes::from_static(&[0x01])] {
            let empty = output.is_empty();
            asserter.push_success(&"0x1");
            asserter.push_success(&output);
            let error = provider
                .resolve_meta_address(&DEPLOYMENT, Address::ZERO)
                .await
                .unwrap_err();
            if empty {
                assert!(matches!(
                    error,
                    ContractError::Contract(alloy::contract::Error::ZeroData(..))
                ));
            } else {
                assert!(matches!(
                    error,
                    ContractError::Contract(alloy::contract::Error::AbiError(_))
                ));
            }
        }
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn invalid_meta_address() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        for bytes in [Bytes::from(vec![0; 1_249]), vec![0; 1_250].into()] {
            asserter.push_success(&"0x1");
            let encoded = IERC6538Registry::stealthMetaAddressOfCall::abi_encode_returns(&bytes);
            asserter.push_success(&Bytes::from(encoded));
            let error = provider
                .resolve_meta_address(&DEPLOYMENT, Address::ZERO)
                .await
                .unwrap_err();
            assert!(matches!(error, ContractError::InvalidMetaAddress));
        }
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn rpc_errors() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_failure_msg("unavailable");
        let error = provider
            .resolve_meta_address(&DEPLOYMENT, Address::ZERO)
            .await
            .unwrap_err();
        assert!(matches!(error, ContractError::Rpc(_)));

        asserter.push_success(&"0x1");
        asserter.push_failure_msg("unavailable");
        let error = provider
            .resolve_meta_address(&DEPLOYMENT, Address::ZERO)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ContractError::Contract(alloy::contract::Error::TransportError(_))
        ));
        assert!(asserter.read_q().is_empty());
    }
}
