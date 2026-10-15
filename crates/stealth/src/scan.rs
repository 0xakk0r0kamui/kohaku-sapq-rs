use std::ops::RangeInclusive;

use alloy::primitives::Address;
use futures::{Stream, TryStreamExt, stream};

use crate::{
    scheme3::{Match, Scanner, Scheme3Account, SchemeError, SpendKey, scan_announcement},
    syncer::{AnnouncementRecord, FetchEvent},
};

#[derive(Debug)]
pub struct MatchedStealthAddress {
    record: AnnouncementRecord,
    match_result: Match,
}

impl MatchedStealthAddress {
    #[must_use]
    pub const fn record(&self) -> &AnnouncementRecord {
        &self.record
    }

    #[must_use]
    pub const fn match_result(&self) -> &Match {
        &self.match_result
    }

    #[must_use]
    pub fn stealth_address(&self) -> Address {
        Address::from(self.match_result.stealth_address)
    }

    /// Derives the key for this address. Fails if `account` does not own the address.
    pub fn spend_key(&self, account: &Scheme3Account) -> Result<SpendKey, SchemeError> {
        account.spend_key(&self.match_result)
    }
}

#[must_use]
pub fn scan_record(scanner: &Scanner, record: AnnouncementRecord) -> Option<MatchedStealthAddress> {
    let match_result = scan_announcement(scanner, record.announcement())?;
    Some(MatchedStealthAddress {
        record,
        match_result,
    })
}

#[derive(Debug)]
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
/// A source error is forwarded and ends the stream.
pub fn get_announcements_for_user<'a, S, E>(
    scanner: &'a Scanner,
    source: S,
) -> impl Stream<Item = Result<ScanEvent, E>> + 'a
where
    S: Stream<Item = Result<FetchEvent, E>> + 'a,
    E: 'a,
{
    stream::try_unfold(Box::pin(source), move |mut source| async move {
        while let Some(event) = source.try_next().await? {
            let event = match event {
                FetchEvent::Announcement(record) => {
                    let Some(matched) = scan_record(scanner, record) else {
                        continue;
                    };
                    ScanEvent::Match(matched)
                }
                FetchEvent::RangeFetched {
                    range,
                    rejected_logs,
                } => ScanEvent::RangeScanned {
                    range,
                    rejected_logs,
                },
            };
            return Ok(Some((event, source)));
        }
        Ok(None)
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use alloy::primitives::B256;
    use futures::{StreamExt, pin_mut};
    use rand::{SeedableRng, rngs::StdRng};

    use super::*;
    use crate::generate_stealth_address;

    #[tokio::test]
    async fn matches_before_progress() {
        let account = Scheme3Account::from_keygen_master(&[7; 32])
            .unwrap()
            .account;
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
            AnnouncementRecord::new(
                announcement,
                Address::ZERO,
                0,
                B256::ZERO,
                B256::ZERO,
                index,
            )
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
            assert_eq!(matched.record().stealth_address(), address);
            let debug = format!("{matched:?}");
            let secret = &matched.match_result().shared_secret;
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
    async fn source_error_stops_scan() {
        let account = Scheme3Account::from_keygen_master(&[7; 32])
            .unwrap()
            .account;
        let scanner = account.scanner().unwrap();
        let events = stream::iter([
            Err("endpoint failed"),
            Ok(FetchEvent::RangeFetched {
                range: 0..=1,
                rejected_logs: 0,
            }),
        ]);
        let events = get_announcements_for_user(&scanner, events);
        pin_mut!(events);
        assert_eq!(events.next().await.unwrap().unwrap_err(), "endpoint failed");
        assert!(events.next().await.is_none());
    }
}
