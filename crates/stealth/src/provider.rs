use std::ops::RangeInclusive;

use alloy::{primitives::Address, providers::DynProvider, rpc::types::TransactionRequest};
use kohaku_kv_store::Store;
use thiserror::Error;
use tracing::debug;

use crate::{
    contracts::{self, ContractError, Deployment},
    indexer::{AnnouncementIndexer, IndexerConfig, IndexerError, SyncReport},
    payment::PaymentBuilder,
    scheme3::{
        Announcement, MatchedAnnouncement, Scanner, Scheme3Account, SchemeError,
        StealthMetaAddress, StealthPrivateKey, try_match_announcement,
    },
    syncer::{AnnouncementRecord, AnnouncementSyncer, RpcAnnouncementSyncer},
};

#[derive(Debug, Error)]
pub enum StealthProviderError {
    #[error(transparent)]
    Contract(#[from] ContractError),
    #[error(transparent)]
    Indexer(#[from] IndexerError),
    #[error(transparent)]
    Scheme(#[from] SchemeError),
}

#[derive(Debug)]
pub struct MatchedPayment {
    record: AnnouncementRecord,
    match_result: MatchedAnnouncement,
}

impl MatchedPayment {
    #[must_use]
    pub fn record(&self) -> &AnnouncementRecord {
        &self.record
    }

    #[must_use]
    pub fn match_result(&self) -> &MatchedAnnouncement {
        &self.match_result
    }

    #[must_use]
    pub fn stealth_address(&self) -> Address {
        self.match_result.stealth_address()
    }

    /// Derives this payment's one-time private key.
    ///
    /// # Errors
    ///
    /// Returns an error if `account` does not control the matched stealth address.
    pub fn derive_stealth_private_key(
        &self,
        account: &Scheme3Account,
    ) -> Result<StealthPrivateKey, SchemeError> {
        account.derive_stealth_private_key(&self.match_result)
    }
}

#[derive(Clone)]
pub struct StealthProvider {
    provider: DynProvider,
    deployment: Deployment,
    indexer: AnnouncementIndexer,
}

impl StealthProvider {
    #[must_use]
    pub fn new(
        store: &Store,
        syncer: impl Into<AnnouncementSyncer>,
        provider: DynProvider,
        deployment: Deployment,
    ) -> Self {
        let indexer =
            AnnouncementIndexer::new(store, syncer.into(), deployment, IndexerConfig::default());
        Self {
            provider,
            deployment,
            indexer,
        }
    }

    #[must_use]
    pub fn rpc(store: &Store, provider: DynProvider, deployment: Deployment) -> Self {
        let syncer = RpcAnnouncementSyncer::new(provider.clone(), deployment);
        Self::new(store, syncer, provider, deployment)
    }

    #[must_use]
    pub fn with_indexer_config(mut self, config: IndexerConfig) -> Self {
        self.indexer = self.indexer.with_config(config);
        self
    }

    #[must_use]
    pub fn prepare_registration(&self, meta_address: &StealthMetaAddress) -> TransactionRequest {
        contracts::prepare_registration(&self.deployment, meta_address)
    }

    #[must_use]
    pub fn prepare_registration_on_behalf(
        &self,
        registrant: Address,
        signature: &[u8],
        meta_address: &StealthMetaAddress,
    ) -> TransactionRequest {
        contracts::prepare_registration_on_behalf(
            &self.deployment,
            registrant,
            signature,
            meta_address,
        )
    }

    #[must_use]
    pub fn prepare_announcement(&self, announcement: &Announcement) -> TransactionRequest {
        contracts::prepare_announcement(&self.deployment, announcement)
    }

    /// Resolves and validates a registrant's scheme 3 meta-address.
    ///
    /// # Errors
    ///
    /// Returns an error when the registry call fails or returns invalid scheme 3 data.
    pub async fn resolve_meta_address(
        &self,
        registrant: Address,
    ) -> Result<StealthMetaAddress, StealthProviderError> {
        Ok(contracts::resolve_meta_address(&self.provider, &self.deployment, registrant).await?)
    }

    #[must_use]
    pub const fn payment<'a>(&self, recipient: &'a StealthMetaAddress) -> PaymentBuilder<'a> {
        PaymentBuilder::new(self.deployment, recipient)
    }

    /// Synchronizes the local announcement index.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured source or local storage fails.
    pub async fn sync(&self) -> Result<SyncReport, StealthProviderError> {
        Ok(self.indexer.sync().await?)
    }

    /// Returns the validated announcements held in the local index.
    ///
    /// # Errors
    ///
    /// Returns an error when local storage cannot be read or contains invalid data.
    pub async fn announcements(&self) -> Result<Vec<AnnouncementRecord>, StealthProviderError> {
        Ok(self.indexer.announcements().await?)
    }

    /// Returns validated announcements in an inclusive block range.
    ///
    /// # Errors
    ///
    /// Returns an error when local storage cannot be read or contains invalid data.
    pub async fn announcements_in(
        &self,
        range: RangeInclusive<u64>,
    ) -> Result<Vec<AnnouncementRecord>, StealthProviderError> {
        Ok(self.indexer.announcements_in(range).await?)
    }

    /// Scans locally indexed announcements with delegated scan material.
    ///
    /// # Errors
    ///
    /// Returns an error when local storage cannot be read or contains invalid data.
    pub async fn matches(
        &self,
        scanner: &Scanner,
    ) -> Result<Vec<MatchedPayment>, StealthProviderError> {
        let records = self.indexer.announcements().await?;
        scan_records(scanner, records)
    }

    /// Scans locally indexed announcements in an inclusive block range.
    ///
    /// # Errors
    ///
    /// Returns an error when local storage cannot be read or contains invalid data.
    pub async fn matches_in(
        &self,
        scanner: &Scanner,
        range: RangeInclusive<u64>,
    ) -> Result<Vec<MatchedPayment>, StealthProviderError> {
        let records = self.indexer.announcements_in(range).await?;
        scan_records(scanner, records)
    }
}

fn scan_records(
    scanner: &Scanner,
    records: Vec<AnnouncementRecord>,
) -> Result<Vec<MatchedPayment>, StealthProviderError> {
    let examined = records.len();
    let mut matches = Vec::new();
    for record in records {
        let announcement = record.announcement()?;
        if let Some(matched) = try_match_announcement(scanner, &announcement) {
            matches.push(MatchedPayment {
                record,
                match_result: matched,
            });
        }
    }
    debug!(
        examined,
        matched = matches.len(),
        "scanned stealth announcements"
    );
    Ok(matches)
}
