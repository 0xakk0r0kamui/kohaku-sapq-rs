use alloy::{
    primitives::{Address, U256},
    providers::DynProvider,
    rpc::types::TransactionRequest,
};
use kohaku_kv_store::{Store, backend::StoreError};
use pqsa_core::SenderState;
use thiserror::Error;
use tracing::{info, warn};

use crate::{
    chain::{Asset, Chain, ChainError, Deployment},
    scheme::{self, Announcement, Match, SCHEME_ID, SchemeError, Tracking},
};

#[derive(Clone, Copy, Debug)]
pub struct Transfer {
    pub asset: Asset,
    pub amount: U256,
}

#[derive(Debug)]
pub struct Payment {
    pub stealth_address: Address,
    pub announce: TransactionRequest,
    pub fund: TransactionRequest,
}

#[derive(Debug, Error)]
pub enum Scheme3Error {
    #[error(transparent)]
    Scheme(#[from] SchemeError),
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error("sender index advanced to {next_index} but was not stored")]
    IndexNotStored {
        next_index: u64,
        #[source]
        source: StoreError,
    },
    #[error("deployment scheme id {got} is not {SCHEME_ID}")]
    WrongScheme { got: u64 },
}

#[derive(Clone)]
pub struct Scheme3Client {
    senders: Store,
    chain: Chain,
}

impl Scheme3Client {
    /// # Errors
    ///
    /// [`Scheme3Error::WrongScheme`] when `deployment.scheme_id` is not [`SCHEME_ID`].
    pub fn new(
        store: &Store,
        provider: DynProvider,
        deployment: Deployment,
    ) -> Result<Self, Scheme3Error> {
        if deployment.scheme_id != SCHEME_ID {
            return Err(Scheme3Error::WrongScheme {
                got: deployment.scheme_id,
            });
        }
        Ok(Self {
            senders: store.scope("kohaku-stealth/senders"),
            chain: Chain::new(store, provider, deployment),
        })
    }

    /// # Errors
    ///
    /// See [`Chain::sync`].
    pub async fn sync(&self) -> Result<crate::chain::SyncReport, Scheme3Error> {
        Ok(self.chain.sync().await?)
    }

    #[must_use]
    pub fn register_keys_tx(&self, meta_address: &[u8]) -> TransactionRequest {
        self.chain.register_keys_tx(meta_address)
    }

    /// Unsigned `announce` for an announcement already built.
    ///
    /// # Errors
    ///
    /// [`SchemeError::Malformed`] when `announcement.scheme_id` is not [`SCHEME_ID`]
    /// or the field lengths are not 33 and 1 089.
    pub fn announce_tx(
        &self,
        announcement: &Announcement,
    ) -> Result<TransactionRequest, Scheme3Error> {
        if announcement.scheme_id != SCHEME_ID
            || !scheme::announcement_shape_ok(
                &announcement.ephemeral_pub_key,
                &announcement.metadata,
            )
        {
            warn!(
                scheme_id = announcement.scheme_id,
                ephemeral_pub_key_len = announcement.ephemeral_pub_key.len(),
                metadata_len = announcement.metadata.len(),
                "reject announce transaction"
            );
            return Err(SchemeError::Malformed.into());
        }
        info!(
            stealth_address = %Address::from(announcement.stealth_address),
            ephemeral_pub_key_len = announcement.ephemeral_pub_key.len(),
            metadata_len = announcement.metadata.len(),
            "announce transaction"
        );
        Ok(self.chain.announce_tx(
            Address::from(announcement.stealth_address),
            &announcement.ephemeral_pub_key,
            &announcement.metadata,
        ))
    }

    /// Scheme 3 announcements stored by [`Self::sync`].
    ///
    /// # Errors
    ///
    /// The stored log could not be decoded.
    pub async fn announcements(&self) -> Result<Vec<crate::Record>, Scheme3Error> {
        let records = self.chain.records().await?;
        info!(count = records.len(), "stored announcements");
        Ok(records)
    }

    /// # Errors
    ///
    /// See [`Chain::meta_address_of`].
    pub async fn meta_address_of(&self, registrant: Address) -> Result<Vec<u8>, Scheme3Error> {
        Ok(self.chain.meta_address_of(registrant).await?)
    }

    /// Build the announcement and the funding transfer.
    ///
    /// # Errors
    ///
    /// Scheme failures, or [`Scheme3Error::IndexNotStored`] when the new index
    /// could not be written. The announcement is not returned in that case.
    pub async fn prepare_payment(
        &self,
        recipient_meta: &[u8],
        sender_master: [u8; 32],
        transfer: Transfer,
    ) -> Result<Payment, Scheme3Error> {
        let _guard = prepare_lock().lock().await;
        let counter = self.load_counter(&sender_master).await?;
        let mut sender = SenderState::resume(sender_master, counter);
        let announced = scheme::announce(recipient_meta, &mut sender);
        let next_index = sender.counter();
        self.store_counter(&sender_master, next_index).await?;
        let wire = announced?;
        info!(
            next_sender_index = next_index,
            stealth_address = %Address::from(wire.stealth_address),
            "sender index stored"
        );
        Ok(payment_from_wire(&self.chain, &wire, transfer))
    }

    /// # Errors
    ///
    /// [`SchemeError::TrackingKeyMismatch`] or [`SchemeError::Malformed`] from
    /// binding the scanner. Individual logs that are not ours are omitted.
    pub async fn scan(
        &self,
        tracking: &Tracking,
        meta_address: &[u8],
    ) -> Result<Vec<Match>, Scheme3Error> {
        let scanner = scheme::bind(tracking, meta_address)?;
        let records = self.chain.records().await?;
        let examined = records.len();
        let mut found = Vec::new();
        let mut seen = std::collections::HashSet::<[u8; 20]>::new();
        let mut skipped = 0usize;
        for record in &records {
            match scheme::check(
                &scanner,
                record.scheme_id,
                &record.stealth_address,
                &record.ephemeral_pub_key,
                &record.metadata,
            ) {
                Some(payment) if seen.insert(payment.stealth_address) => found.push(payment),
                Some(_) | None => skipped += 1,
            }
        }
        info!(examined, matched = found.len(), skipped, "scheme 3 scan");
        Ok(found)
    }
}

fn payment_from_wire(chain: &Chain, wire: &Announcement, transfer: Transfer) -> Payment {
    let stealth_address = Address::from(wire.stealth_address);
    let announce = chain.announce_tx(stealth_address, &wire.ephemeral_pub_key, &wire.metadata);
    let fund = Chain::fund_tx(transfer.asset, stealth_address, transfer.amount);
    Payment {
        stealth_address,
        announce,
        fund,
    }
}

impl Scheme3Client {
    async fn load_counter(&self, sender_master: &[u8; 32]) -> Result<u64, Scheme3Error> {
        let Some(bytes) = self
            .senders
            .get(scheme::sender_counter_key(sender_master))
            .await
            .map_err(ChainError::from)?
        else {
            return Ok(0);
        };
        postcard::from_bytes(&bytes)
            .map_err(ChainError::CorruptStore)
            .map_err(Scheme3Error::from)
    }

    async fn store_counter(
        &self,
        sender_master: &[u8; 32],
        counter: u64,
    ) -> Result<(), Scheme3Error> {
        let bytes = postcard::to_allocvec(&counter).map_err(|err| Scheme3Error::IndexNotStored {
            next_index: counter,
            source: StoreError::from(Box::new(err) as Box<dyn std::error::Error + Send + Sync>),
        })?;
        self.senders
            .put(scheme::sender_counter_key(sender_master), bytes)
            .await
            .map_err(|source| Scheme3Error::IndexNotStored {
                next_index: counter,
                source,
            })?;
        Ok(())
    }
}

fn prepare_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}
