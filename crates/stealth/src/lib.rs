#![doc = include_str!("../README.md")]

mod abis;
pub mod contracts;
mod provider;
pub mod scan;
pub mod scheme3;
pub mod syncer;

pub use contracts::Deployment;
pub use kohaku_rpc_provider::StreamedEventConfig;
pub use provider::{ContractError, StealthProviderExt};
pub use scan::{MatchedStealthAddress, ScanEvent, get_announcements_for_user, scan_record};
pub use scheme3::{
    Account, AccountSeed, DerivedAccount, EngineError, ExposeSecret, Master, Match, Scanner,
    SchemeError, SchemeId3, SpendKey, StealthAddress, StealthMetaAddress, StealthScheme, Tracking,
    generate_stealth_address,
};
pub use syncer::{AnnouncementRecord, FetchError, FetchEvent, get_announcements};
