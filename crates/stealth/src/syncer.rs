use std::ops::RangeInclusive;

use alloy::{
    primitives::{Address, B256, Bytes, U256},
    providers::Provider,
    rpc::types::{BlockNumberOrTag, Filter, Log},
    sol_types::SolEvent,
    transports::TransportError,
};
use futures::{Stream, TryStreamExt, stream};
use kohaku_rpc_provider::{StreamedEvent, StreamedEventConfig};
use pqsa_core::StealthScheme;
use pqsa_per_payment::SchemeId3;
use thiserror::Error;

use crate::{
    abis::IERC5564Announcer::Announcement as AnnouncementEvent, contracts::Deployment,
    scheme3::Announcement,
};

/// An announcement from an ERC-5564 log.
#[derive(Clone, Debug)]
pub struct AnnouncementRecord {
    pub announcement: Announcement,
    /// Full ERC-5564 metadata, including the view tag and any sender-supplied data.
    /// The scanner does not validate the application data.
    pub metadata: Bytes,
    pub caller: Address,
    pub block_number: u64,
    pub block_hash: B256,
    pub transaction_hash: B256,
    pub log_index: u64,
}

impl AnnouncementRecord {
    #[must_use]
    pub fn stealth_address(&self) -> Address {
        Address::from(self.announcement.stealth_address)
    }
}

#[derive(Clone, Debug)]
pub enum FetchEvent {
    Announcement(AnnouncementRecord),
    /// Follows all announcements in a completed range, including ranges with no announcements.
    RangeFetched {
        range: RangeInclusive<u64>,
        /// Logs with invalid scheme 3 payloads.
        rejected_logs: usize,
    },
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FetchError {
    #[error(transparent)]
    Rpc(#[from] TransportError),
    #[error("RPC is connected to chain {actual}; deployment expects chain {expected}")]
    WrongChain { expected: u64, actual: u64 },
    #[error("RPC returned no finalized block")]
    MissingFinalizedBlock,
    #[error("invalid announcement event ABI at block {block_number}, index {log_index}")]
    InvalidEvent {
        block_number: u64,
        log_index: u64,
        #[source]
        source: alloy::sol_types::Error,
    },
    #[error("invalid RPC log at block {block_number:?}, index {log_index:?}: {reason}")]
    InvalidLog {
        block_number: Option<u64>,
        log_index: Option<u64>,
        reason: &'static str,
    },
}

/// Streams announcements in block and log order.
/// Uses [`StreamedEvent`] for chunking, concurrency and retries.
/// Both range endpoints are inclusive. The range is limited to `deployment.start_block`
/// through the finalized block read on first poll. Use `from_block..=u64::MAX` to scan to that block.
/// Invalid scheme 3 payloads are skipped and counted in [`FetchEvent::RangeFetched`].
/// Emits one `RangeFetched` for the actual range after all logs are processed, even if none match.
/// If the clamped range is empty, the stream ends without querying logs or emitting a range.
///
/// # Errors
///
/// Yields one error, then ends without `RangeFetched`. Out-of-order and duplicate logs are errors.
/// On error or cancellation, replay the range and deduplicate any logs already received.
pub fn get_announcements(
    provider: &impl Provider,
    deployment: Deployment,
    range: RangeInclusive<u64>,
    config: StreamedEventConfig,
) -> impl Stream<Item = Result<FetchEvent, FetchError>> + '_ {
    stream::once(async move {
        let actual = provider.get_chain_id().await?;
        if actual != deployment.chain_id {
            return Err(FetchError::WrongChain {
                expected: deployment.chain_id,
                actual,
            });
        }
        let head = provider
            .get_block_by_number(BlockNumberOrTag::Finalized)
            .await?
            .ok_or(FetchError::MissingFinalizedBlock)?
            .header
            .number;
        let from = (*range.start()).max(deployment.start_block);
        let to = (*range.end()).min(head);
        let filter = Filter::new()
            .address(deployment.announcer)
            .event_signature(AnnouncementEvent::SIGNATURE_HASH)
            .topic1(B256::from(U256::from(SchemeId3::SCHEME_ID)))
            .from_block(from)
            .to_block(to);
        let logs = Box::pin(
            StreamedEvent::new(provider, filter.clone())
                .config(config)
                .stream(),
        );
        let range = (from <= to).then_some(from..=to);
        Ok::<_, FetchError>(stream::try_unfold(
            (logs, range, filter, 0, None),
            |(mut logs, range, filter, mut rejected_logs, mut previous)| async move {
                let Some(range) = range else {
                    return Ok(None);
                };
                while let Some(log) = logs.try_next().await? {
                    if let Some(record) = decode_log(&log, &filter, &range, &mut previous)? {
                        return Ok(Some((
                            FetchEvent::Announcement(record),
                            (logs, Some(range), filter, rejected_logs, previous),
                        )));
                    }
                    rejected_logs += 1;
                }
                Ok(Some((
                    FetchEvent::RangeFetched {
                        range,
                        rejected_logs,
                    },
                    (logs, None, filter, rejected_logs, previous),
                )))
            },
        ))
    })
    .try_flatten()
}

fn invalid_log(log: &Log, reason: &'static str) -> FetchError {
    FetchError::InvalidLog {
        block_number: log.block_number,
        log_index: log.log_index,
        reason,
    }
}

fn decode_log(
    log: &Log,
    filter: &Filter,
    range: &RangeInclusive<u64>,
    previous: &mut Option<(u64, u64, B256)>,
) -> Result<Option<AnnouncementRecord>, FetchError> {
    let (Some(block_number), Some(block_hash), Some(transaction_hash), Some(log_index)) = (
        log.block_number,
        log.block_hash,
        log.transaction_hash,
        log.log_index,
    ) else {
        return Err(invalid_log(log, "expected a mined, non-removed log"));
    };
    if log.removed {
        return Err(invalid_log(log, "expected a mined, non-removed log"));
    }
    if !range.contains(&block_number) || !filter.matches(&log.inner) {
        return Err(invalid_log(log, "log does not match the requested filter"));
    }
    if let Some((previous_block, previous_index, previous_hash)) = *previous {
        if block_number == previous_block && block_hash != previous_hash {
            return Err(invalid_log(
                log,
                "conflicting hashes for one finalized block",
            ));
        }
        if (block_number, log_index) <= (previous_block, previous_index) {
            return Err(invalid_log(log, "duplicate or out-of-order log"));
        }
    }
    *previous = Some((block_number, log_index, block_hash));
    let decoded =
        AnnouncementEvent::decode_log(&log.inner).map_err(|source| FetchError::InvalidEvent {
            block_number,
            log_index,
            source,
        })?;
    let announcement = SchemeId3::announcement_from_bytes(
        &decoded.data.stealthAddress.into_array(),
        &decoded.data.ephemeralPubKey,
        &decoded.data.metadata,
    );
    Ok(announcement.map(|announcement| AnnouncementRecord {
        announcement,
        metadata: decoded.data.metadata,
        caller: decoded.data.caller,
        block_number,
        block_hash,
        transaction_hash,
        log_index,
    }))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use alloy::{
        providers::{DynProvider, ProviderBuilder},
        rpc::types::Block,
        transports::mock::Asserter,
    };
    use futures::pin_mut;

    use super::*;

    const ANNOUNCER: Address = Address::repeat_byte(0x33);
    const DEPLOYMENT: Deployment = Deployment {
        chain_id: 1,
        announcer: ANNOUNCER,
        registry: Address::ZERO,
        start_block: 0,
    };

    fn mock(head: u64) -> (DynProvider, Asserter) {
        let asserter = Asserter::new();
        asserter.push_success(&"0x1");
        let mut block = Block::<alloy::rpc::types::Transaction>::default();
        block.header.number = head;
        asserter.push_success(&block);
        let provider = ProviderBuilder::new()
            .connect_mocked_client(asserter.clone())
            .erased();
        (provider, asserter)
    }

    fn log(block: u64, index: u64) -> Log {
        let event = AnnouncementEvent {
            schemeId: U256::from(SchemeId3::SCHEME_ID),
            stealthAddress: Address::repeat_byte(0xaa),
            caller: Address::repeat_byte(0xbb),
            ephemeralPubKey: vec![0x02; pqsa_per_payment::EPHEMERAL_PUB_KEY_BYTES].into(),
            metadata: vec![0].into(),
        };
        Log {
            inner: alloy::primitives::Log {
                address: ANNOUNCER,
                data: event.encode_log_data(),
            },
            block_number: Some(block),
            block_hash: Some(B256::repeat_byte(0x44)),
            transaction_hash: Some(B256::repeat_byte(0x55)),
            log_index: Some(index),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn fetch_range_limits() {
        let (provider, asserter) = mock(5);
        asserter.push_success(&Vec::<Log>::new());
        asserter.push_success(&Vec::<Log>::new());
        let events = get_announcements(
            &provider,
            Deployment {
                start_block: 3,
                ..DEPLOYMENT
            },
            0..=u64::MAX,
            StreamedEventConfig {
                chunk_size: 2,
                ..Default::default()
            },
        )
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
        assert!(matches!(events.as_slice(), [
            FetchEvent::RangeFetched { range, rejected_logs: 0 },
        ] if range == &(3..=5)));
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn skips_bad_payloads() {
        let (provider, asserter) = mock(3);
        let mut logs = vec![log(1, 0)];
        for (index, epk, metadata) in [
            (1, vec![0x02; 1120], vec![0]),
            (2, vec![0; 1121], vec![0]),
            (3, vec![0x02; 1122], vec![0]),
            (4, vec![0x02; 1121], vec![]),
        ] {
            let mut malformed = log(1, index);
            let event = AnnouncementEvent {
                schemeId: U256::from(SchemeId3::SCHEME_ID),
                stealthAddress: Address::ZERO,
                caller: Address::ZERO,
                ephemeralPubKey: epk.into(),
                metadata: metadata.into(),
            };
            malformed.inner.data = event.encode_log_data();
            logs.push(malformed);
        }
        logs.push(log(2, 0));
        asserter.push_success(&logs);
        let events =
            get_announcements(&provider, DEPLOYMENT, 1..=2, StreamedEventConfig::default())
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
        let positions: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                FetchEvent::Announcement(record) => Some((record.block_number, record.log_index)),
                FetchEvent::RangeFetched { .. } => None,
            })
            .collect();
        assert_eq!(positions, vec![(1, 0), (2, 0)]);
        assert!(matches!(events.last(),
            Some(FetchEvent::RangeFetched { range, rejected_logs: 4 }) if range == &(1..=2)));
    }

    #[tokio::test]
    async fn empty_clamped_range() {
        for (requested, start_block) in [
            (6..=u64::MAX, 0),
            (0..=2, 3),
            (RangeInclusive::new(3, 2), 0),
        ] {
            let (provider, asserter) = mock(5);
            let events = get_announcements(
                &provider,
                Deployment {
                    start_block,
                    ..DEPLOYMENT
                },
                requested,
                StreamedEventConfig::default(),
            )
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
            assert!(events.is_empty());
            assert!(asserter.read_q().is_empty());
        }
    }

    #[tokio::test]
    async fn all_payloads_rejected() {
        let (provider, asserter) = mock(1);
        let mut invalid = log(1, 0);
        let mut event = AnnouncementEvent::decode_log(&invalid.inner).unwrap().data;
        event.metadata = Bytes::new();
        invalid.inner.data = event.encode_log_data();
        asserter.push_success(&vec![invalid]);
        let events =
            get_announcements(&provider, DEPLOYMENT, 1..=1, StreamedEventConfig::default())
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
        assert!(matches!(events.as_slice(), [
            FetchEvent::RangeFetched { range, rejected_logs: 1 },
        ] if range == &(1..=1)));
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn invalid_log_sequence() {
        let mut conflicting = log(1, 1);
        conflicting.block_hash = Some(B256::ZERO);
        for (first, second, reason) in [
            (log(1, 0), log(1, 0), "duplicate or out-of-order log"),
            (log(1, 1), log(1, 0), "duplicate or out-of-order log"),
            (log(2, 0), log(1, 0), "duplicate or out-of-order log"),
            (
                log(1, 0),
                conflicting,
                "conflicting hashes for one finalized block",
            ),
        ] {
            let (provider, asserter) = mock(2);
            asserter.push_success(&vec![first, second]);
            let events =
                get_announcements(&provider, DEPLOYMENT, 1..=2, StreamedEventConfig::default());
            pin_mut!(events);
            assert!(matches!(
                events.try_next().await.unwrap(),
                Some(FetchEvent::Announcement(_))
            ));
            assert!(matches!(
                events.try_next().await,
                Err(FetchError::InvalidLog { reason: actual, .. }) if actual == reason
            ));
            assert!(events.try_next().await.unwrap().is_none());
            assert!(asserter.read_q().is_empty());
        }
    }

    #[tokio::test]
    async fn rejected_payload_still_checks_order() {
        let (provider, asserter) = mock(1);
        let mut invalid = log(1, 1);
        let mut event = AnnouncementEvent::decode_log(&invalid.inner).unwrap().data;
        event.metadata = Bytes::new();
        invalid.inner.data = event.encode_log_data();
        asserter.push_success(&vec![invalid, log(1, 0)]);
        let events =
            get_announcements(&provider, DEPLOYMENT, 1..=1, StreamedEventConfig::default());
        pin_mut!(events);
        assert!(matches!(
            events.try_next().await,
            Err(FetchError::InvalidLog {
                reason: "duplicate or out-of-order log",
                ..
            })
        ));
        assert!(events.try_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn dropping_stream_stops_fetching() {
        let (provider, asserter) = mock(1);
        asserter.push_success(&vec![log(0, 0)]);
        asserter.push_success(&vec![log(1, 0)]);
        let mut events = Box::pin(get_announcements(
            &provider,
            DEPLOYMENT,
            0..=1,
            StreamedEventConfig {
                chunk_size: 1,
                max_concurrent: 1,
                ..Default::default()
            },
        ));
        assert!(matches!(
            events.try_next().await.unwrap(),
            Some(FetchEvent::Announcement(_))
        ));
        drop(events);
        assert_eq!(asserter.read_q().len(), 1);
    }

    #[tokio::test]
    async fn wrong_chain() {
        let (provider, asserter) = mock(2);
        let events = get_announcements(
            &provider,
            Deployment {
                chain_id: 2,
                ..DEPLOYMENT
            },
            0..=2,
            StreamedEventConfig::default(),
        );
        pin_mut!(events);
        assert!(matches!(
            events.try_next().await,
            Err(FetchError::WrongChain {
                expected: 2,
                actual: 1
            })
        ));
        assert!(events.try_next().await.unwrap().is_none());
        assert_eq!(
            asserter.read_q().len(),
            1,
            "head response must remain unused"
        );
    }

    #[tokio::test]
    async fn completes_range_after_reduced_queries() {
        let (provider, asserter) = mock(1);
        asserter.push_failure_msg("range too large");
        asserter.push_success(&vec![log(0, 0)]);
        asserter.push_success(&vec![log(1, 0)]);
        let events =
            get_announcements(&provider, DEPLOYMENT, 0..=1, StreamedEventConfig::default())
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
        assert!(matches!(events.as_slice(), [
            FetchEvent::Announcement(first),
            FetchEvent::Announcement(second),
            FetchEvent::RangeFetched { range, rejected_logs: 0 },
        ] if first.block_number == 0 && second.block_number == 1 && range == &(0..=1)));
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn bad_event_stops_stream() {
        let (provider, asserter) = mock(0);
        let mut invalid = log(0, 1);
        invalid.inner.data = alloy::primitives::LogData::new_unchecked(
            invalid.topics().to_vec(),
            alloy::primitives::Bytes::default(),
        );
        asserter.push_success(&vec![log(0, 0), invalid]);
        let events =
            get_announcements(&provider, DEPLOYMENT, 0..=0, StreamedEventConfig::default());
        pin_mut!(events);
        assert!(matches!(
            events.try_next().await,
            Ok(Some(FetchEvent::Announcement(_)))
        ));
        assert!(matches!(
            events.try_next().await,
            Err(FetchError::InvalidEvent { .. })
        ));
        assert!(events.try_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn failed_range_stops_stream() {
        let (provider, asserter) = mock(2);
        asserter.push_success(&vec![log(0, 0)]);
        let mut invalid = log(1, 0);
        invalid.transaction_hash = None;
        asserter.push_success(&vec![invalid]);
        asserter.push_success(&vec![log(2, 0)]);
        let events = get_announcements(
            &provider,
            DEPLOYMENT,
            0..=2,
            StreamedEventConfig {
                chunk_size: 1,
                max_concurrent: 1,
                ..Default::default()
            },
        );
        pin_mut!(events);
        assert!(matches!(events.try_next().await.unwrap(),
            Some(FetchEvent::Announcement(record)) if record.block_number == 0));
        assert!(matches!(
            events.try_next().await,
            Err(FetchError::InvalidLog { .. })
        ));
        assert!(events.try_next().await.unwrap().is_none());
        assert_eq!(asserter.read_q().len(), 1);
    }

    #[test]
    fn invalid_logs() {
        let valid = log(1, 0);
        let mut missing_block_number = valid.clone();
        missing_block_number.block_number = None;
        let mut missing_block_hash = valid.clone();
        missing_block_hash.block_hash = None;
        let mut missing_transaction_hash = valid.clone();
        missing_transaction_hash.transaction_hash = None;
        let mut missing_log_index = valid.clone();
        missing_log_index.log_index = None;
        let mut removed = valid.clone();
        removed.removed = true;
        let mut foreign = valid.clone();
        foreign.inner.address = Address::ZERO;
        let mut other_event = valid.clone();
        other_event.topics_mut()[0] = B256::ZERO;
        let mut other_scheme = valid.clone();
        other_scheme.topics_mut()[1] = B256::ZERO;
        let filter = Filter::new()
            .address(ANNOUNCER)
            .event_signature(AnnouncementEvent::SIGNATURE_HASH)
            .topic1(B256::from(U256::from(SchemeId3::SCHEME_ID)));
        for log in [
            missing_block_number,
            missing_block_hash,
            missing_transaction_hash,
            missing_log_index,
            removed,
            foreign,
            log(2, 0),
            other_event,
            other_scheme,
        ] {
            assert!(matches!(
                decode_log(&log, &filter, &(1..=1), &mut None),
                Err(FetchError::InvalidLog { .. })
            ));
        }
    }

    #[tokio::test]
    async fn rpc_error_after_announcement() {
        let (provider, asserter) = mock(3);
        asserter.push_success(&vec![log(0, 0)]);
        asserter.push_failure(alloy::rpc::json_rpc::ErrorPayload {
            code: 429,
            message: "rate limited".into(),
            data: None,
        });
        let events = get_announcements(
            &provider,
            DEPLOYMENT,
            0..=3,
            StreamedEventConfig {
                chunk_size: 2,
                max_concurrent: 1,
                max_retries: 0,
                ..Default::default()
            },
        );
        pin_mut!(events);
        assert!(
            matches!(events.try_next().await.unwrap(), Some(FetchEvent::Announcement(record)) if record.block_number == 0)
        );
        let Err(FetchError::Rpc(error)) = events.try_next().await else {
            panic!("expected RPC error without range completion");
        };
        assert_eq!(error.as_error_resp().unwrap().code, 429);
        assert!(events.try_next().await.unwrap().is_none());
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn rejects_logs_outside_range() {
        let (provider, asserter) = mock(5);
        asserter.push_success(&vec![log(4, 0)]);
        let events =
            get_announcements(&provider, DEPLOYMENT, 0..=3, StreamedEventConfig::default());
        pin_mut!(events);
        assert!(matches!(
            events.try_next().await,
            Err(FetchError::InvalidLog { .. })
        ));
        assert!(events.try_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn missing_finalized_block() {
        let asserter = Asserter::new();
        asserter.push_success(&"0x1");
        asserter.push_success(&Option::<Block>::None);
        let provider = ProviderBuilder::new()
            .connect_mocked_client(asserter)
            .erased();
        let events =
            get_announcements(&provider, DEPLOYMENT, 0..=0, StreamedEventConfig::default());
        pin_mut!(events);
        assert!(matches!(
            events.try_next().await,
            Err(FetchError::MissingFinalizedBlock)
        ));
        assert!(events.try_next().await.unwrap().is_none());
    }
}
