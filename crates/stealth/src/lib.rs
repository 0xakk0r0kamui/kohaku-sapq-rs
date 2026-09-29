#![doc = include_str!("../README.md")]

pub mod contracts;
pub mod indexer;
pub mod payment;
pub mod provider;
mod scheme;
pub mod scheme3;
pub mod syncer;

pub use contracts::Deployment;
pub use indexer::{IndexerConfig, SyncReport};
pub use payment::{PaymentBuilder, PaymentError, PreparedPayment};
pub use provider::{MatchedPayment, StealthProvider, StealthProviderError};
pub use scheme3::{
    AccountSeed, Announcement, DerivedScheme3Account, GeneratedStealthAddress, MasterKey,
    MatchedAnnouncement, SCHEME_ID, Scanner, Scheme3Account, SchemeError, StealthMetaAddress,
    StealthPrivateKey, TrackingKey,
};
pub use syncer::{
    AnnouncementRecord, AnnouncementSyncer, AnnouncementSyncerBackend, FetchResult,
    RpcAnnouncementSyncer, SyncerError,
};
