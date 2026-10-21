use std::ops::RangeInclusive;

use alloy::primitives::Address;
use futures::{Stream, TryStreamExt, future};

use crate::{
    scheme3::{Match, Scanner, SchemeId3, StealthScheme},
    syncer::{AnnouncementRecord, FetchEvent},
};

#[derive(Clone, Debug)]
pub struct MatchedStealthAddress {
    pub record: AnnouncementRecord,
    /// Contains the shared secret for this payment.
    pub match_result: Match,
}

impl MatchedStealthAddress {
    #[must_use]
    pub fn stealth_address(&self) -> Address {
        Address::from(self.match_result.stealth_address)
    }
}

#[must_use]
pub fn scan_record(scanner: &Scanner, record: AnnouncementRecord) -> Option<MatchedStealthAddress> {
    let match_result = SchemeId3::scan(scanner, &record.announcement)?;
    Some(MatchedStealthAddress {
        record,
        match_result,
    })
}

#[derive(Clone, Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "Avoid a heap allocation for each match."
)]
pub enum ScanEvent {
    Match(MatchedStealthAddress),
    /// Follows the range's matches. Also emitted for ranges with no matches.
    RangeScanned {
        range: RangeInclusive<u64>,
        rejected_logs: usize,
    },
}

/// Returns matches from an announcement stream, followed by [`ScanEvent::RangeScanned`] events.
/// Matching runs on the thread polling the stream.
/// Source errors are forwarded; the source determines whether scanning can continue.
pub fn get_announcements_for_user<'a, S, E>(
    scanner: &'a Scanner,
    source: S,
) -> impl Stream<Item = Result<ScanEvent, E>> + 'a
where
    S: Stream<Item = Result<FetchEvent, E>> + 'a,
    E: 'a,
{
    source.try_filter_map(move |event| {
        future::ready(Ok(match event {
            FetchEvent::Announcement(record) => scan_record(scanner, record).map(ScanEvent::Match),
            FetchEvent::RangeFetched {
                range,
                rejected_logs,
            } => Some(ScanEvent::RangeScanned {
                range,
                rejected_logs,
            }),
        }))
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use alloy::primitives::B256;
    use futures::{StreamExt, pin_mut, stream};
    use rand::{SeedableRng, rngs::StdRng};

    use super::*;
    use crate::{Account, generate_stealth_address};

    #[tokio::test]
    async fn matches_before_progress() {
        let account = Account::from_keygen_master(&[7; 32]).unwrap().account;
        let scanner = account.scanner().unwrap();
        let mut rng = StdRng::from_seed([0x52; 32]);
        let first = generate_stealth_address(account.meta_address(), &mut rng).unwrap();
        let second = generate_stealth_address(account.meta_address(), &mut rng).unwrap();
        let expected = [
            Address::from(first.stealth_address),
            Address::from(second.stealth_address),
        ];
        assert_ne!(expected[0], expected[1]);
        let mut nonmatch = first.clone();
        nonmatch.stealth_address = [0; 20];
        let record = |announcement, index| {
            let (_, _, metadata) = SchemeId3::announcement_to_bytes(&announcement);
            AnnouncementRecord {
                announcement,
                metadata: metadata.into(),
                caller: Address::ZERO,
                block_number: 0,
                block_hash: B256::ZERO,
                transaction_hash: B256::ZERO,
                log_index: index,
            }
        };
        let events = vec![
            Ok::<_, &'static str>(FetchEvent::Announcement(record(first, 0))),
            Ok(FetchEvent::Announcement(record(nonmatch, 1))),
            Ok(FetchEvent::Announcement(record(second, 2))),
            Ok(FetchEvent::RangeFetched {
                range: 0..=0,
                rejected_logs: 1,
            }),
            Ok(FetchEvent::RangeFetched {
                range: 1..=2,
                rejected_logs: 0,
            }),
        ];
        let events = get_announcements_for_user(&scanner, stream::iter(events));
        pin_mut!(events);
        for address in expected {
            let ScanEvent::Match(matched) = events.try_next().await.unwrap().unwrap() else {
                panic!("expected match before progress");
            };
            assert_eq!(matched.stealth_address(), address);
            assert_eq!(matched.record.stealth_address(), address);
            let debug = format!("{matched:?}");
            let secret = &matched.match_result.shared_secret;
            assert!(!debug.contains(&hex::encode(secret)));
            assert!(!debug.contains(&format!("{secret:?}")));
        }
        assert!(matches!(events.try_next().await.unwrap(),
            Some(ScanEvent::RangeScanned { range, rejected_logs: 1 }) if range == (0..=0)));
        assert!(matches!(events.try_next().await.unwrap(),
            Some(ScanEvent::RangeScanned { range, rejected_logs: 0 }) if range == (1..=2)));
        assert!(events.try_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn source_can_continue_after_error() {
        let account = Account::from_keygen_master(&[7; 32]).unwrap().account;
        let scanner = account.scanner().unwrap();
        let announcement = SchemeId3::announce(account.meta_address(), &[0x44; 64]).unwrap();
        let address = Address::from(announcement.stealth_address);
        let (_, _, metadata) = SchemeId3::announcement_to_bytes(&announcement);
        let events = stream::iter([
            Err("endpoint failed"),
            Ok(FetchEvent::Announcement(AnnouncementRecord {
                announcement,
                metadata: metadata.into(),
                caller: Address::ZERO,
                block_number: 0,
                block_hash: B256::ZERO,
                transaction_hash: B256::ZERO,
                log_index: 0,
            })),
            Ok(FetchEvent::RangeFetched {
                range: 0..=1,
                rejected_logs: 0,
            }),
        ])
        .then(|event| async move { event });
        let events = get_announcements_for_user(&scanner, events);
        pin_mut!(events);
        assert_eq!(events.next().await.unwrap().unwrap_err(), "endpoint failed");
        let Some(Ok(ScanEvent::Match(matched))) = events.next().await else {
            panic!("expected match after source recovery");
        };
        assert_eq!(matched.stealth_address(), address);
        assert!(matches!(events.next().await,
            Some(Ok(ScanEvent::RangeScanned { range, rejected_logs: 0 })) if range == (0..=1)));
        assert!(events.next().await.is_none());
    }
}
