#![doc = include_str!("../README.md")]

pub mod contracts;
pub mod scan;
pub mod scheme3;
pub mod syncer;

pub use contracts::Deployment;
pub use scan::{MatchedStealthAddress, ScanEvent, get_announcements_for_user, scan_record};
pub use scheme3::{
    AccountSeed, DerivedScheme3Account, EngineError, Master, Match, SCHEME_ID, Scanner,
    Scheme3Account, SchemeError, StealthAddress, StealthMetaAddress, StealthPrivateKey, Tracking,
    compute_stealth_spend_key, generate_stealth_address, generate_stealth_address_with_seed,
    scan_announcement,
};
pub use syncer::{AnnouncementRecord, FetchConfig, FetchError, FetchEvent, get_announcements};
