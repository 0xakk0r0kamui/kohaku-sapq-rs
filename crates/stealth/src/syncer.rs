use std::{
    num::{NonZeroU64, NonZeroUsize},
    ops::RangeInclusive,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use alloy::{
    primitives::{Address, B256, U256},
    providers::Provider,
    rpc::types::{BlockNumberOrTag, Filter, Log},
    sol_types::SolEvent,
    transports::TransportError,
};
use futures::{Stream, StreamExt, TryStreamExt, stream};
use pqsa_core::StealthScheme;
use pqsa_per_payment::SchemeId3;
use thiserror::Error;

use crate::{
    contracts::{AnnouncementEvent, Deployment},
    scheme3::{SCHEME_ID, StealthAddress},
};

/// An announcement from an ERC-5564 log.
#[derive(Clone, Debug)]
pub struct AnnouncementRecord {
    announcement: StealthAddress,
    caller: Address,
    block_number: u64,
    block_hash: B256,
    transaction_hash: B256,
    log_index: u64,
}

impl AnnouncementRecord {
    #[must_use]
    pub const fn new(
        announcement: StealthAddress,
        caller: Address,
        block_number: u64,
        block_hash: B256,
        transaction_hash: B256,
        log_index: u64,
    ) -> Self {
        Self {
            announcement,
            caller,
            block_number,
            block_hash,
            transaction_hash,
            log_index,
        }
    }

    #[must_use]
    pub const fn announcement(&self) -> &StealthAddress {
        &self.announcement
    }

    #[must_use]
    pub fn stealth_address(&self) -> Address {
        Address::from(self.announcement.stealth_address)
    }

    #[must_use]
    pub const fn caller(&self) -> Address {
        self.caller
    }

    #[must_use]
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    #[must_use]
    pub const fn block_hash(&self) -> B256 {
        self.block_hash
    }

    #[must_use]
    pub const fn transaction_hash(&self) -> B256 {
        self.transaction_hash
    }

    #[must_use]
    pub const fn log_index(&self) -> u64 {
        self.log_index
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FetchConfig {
    /// Maximum blocks per batch. RPC size limits reduce the query size within a batch.
    pub blocks_per_request: NonZeroU64,
    /// Number of batches to fetch concurrently.
    pub prefetch: NonZeroUsize,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            blocks_per_request: NonZeroU64::new(1_000).unwrap(),
            prefetch: NonZeroUsize::new(2).unwrap(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum FetchEvent {
    Announcement(AnnouncementRecord),
    /// Follows the range's announcements. Also emitted for empty ranges.
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
/// Both range endpoints are inclusive. The range is limited to `deployment.start_block`
/// through the finalized block read on first poll. Use `from_block..=u64::MAX` to scan to that block.
/// Invalid scheme 3 payloads are skipped and counted in [`FetchEvent::RangeFetched`].
///
/// # Errors
///
/// Yields one error, then ends. The failed range has no `RangeFetched` event.
pub fn get_announcements(
    provider: &impl Provider,
    deployment: Deployment,
    range: RangeInclusive<u64>,
    config: FetchConfig,
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
        let block_limit = Arc::new(AtomicU64::new(config.blocks_per_request.get()));
        let windows = stream::iter(block_ranges(from, to, config.blocks_per_request))
            .map(move |range| {
                let block_limit = Arc::clone(&block_limit);
                async move {
                    let logs =
                        fetch_logs(provider, deployment.announcer, &range, &block_limit).await?;
                    Ok::<_, FetchError>(Batch {
                        range,
                        logs: logs.into_iter(),
                        rejected_logs: 0,
                    })
                }
            })
            .buffered(config.prefetch.get());

        Ok(stream::try_unfold(
            (windows, None::<Batch>),
            |(mut windows, mut current)| async move {
                loop {
                    if let Some(batch) = &mut current {
                        if let Some(record) = batch.next_record()? {
                            return Ok(Some((
                                FetchEvent::Announcement(record),
                                (windows, current),
                            )));
                        }
                        let event = FetchEvent::RangeFetched {
                            range: batch.range.clone(),
                            rejected_logs: batch.rejected_logs,
                        };
                        return Ok(Some((event, (windows, None))));
                    }
                    current = windows.try_next().await?;
                    if current.is_none() {
                        return Ok(None);
                    }
                }
            },
        ))
    })
    .try_flatten()
}

async fn fetch_logs(
    provider: &impl Provider,
    announcer: Address,
    range: &RangeInclusive<u64>,
    block_limit: &AtomicU64,
) -> Result<Vec<Log>, FetchError> {
    let mut from = *range.start();
    let to = *range.end();
    let mut logs = Vec::new();
    loop {
        let size = block_limit.load(Ordering::Relaxed);
        let end = from.saturating_add(size - 1).min(to);
        let filter = Filter::new()
            .address(announcer)
            .event_signature(AnnouncementEvent::SIGNATURE_HASH)
            .topic1(B256::from(U256::from(SCHEME_ID)))
            .from_block(from)
            .to_block(end);
        match provider.get_logs(&filter).await {
            Ok(page) => {
                logs.extend(validate_logs(page, announcer, &(from..=end))?);
                if end == to {
                    return Ok(logs);
                }
                from = end + 1;
            }
            Err(error) if end > from && is_log_limit(&error) => {
                let blocks = end - from + 1;
                block_limit.fetch_min(blocks / 2, Ordering::Relaxed);
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn is_log_limit(error: &TransportError) -> bool {
    let Some(error) = error.as_error_resp() else {
        return false;
    };
    if !matches!(error.code, -32000 | -32005 | -32602 | -32603) {
        return false;
    }
    // RPC codes also cover rate limits; require a query-size message.
    let message = error.message.to_ascii_lowercase();
    [
        "query returned more than",
        "too many results",
        "response size exceeded",
        "query timeout exceeded",
        "range too large",
        "block range is too wide",
        "block range limit exceeded",
    ]
    .iter()
    .any(|text| message.contains(text))
        || ((message.contains("block range") || message.contains("blocks range"))
            && message.contains("limited to"))
}

struct Batch {
    range: RangeInclusive<u64>,
    logs: std::vec::IntoIter<Log>,
    rejected_logs: usize,
}

impl Batch {
    fn next_record(&mut self) -> Result<Option<AnnouncementRecord>, FetchError> {
        // validate_logs checked the block and transaction fields.
        for log in self.logs.by_ref() {
            let decoded = AnnouncementEvent::decode_log(&log.inner).map_err(|source| {
                FetchError::InvalidEvent {
                    block_number: log.block_number.unwrap(),
                    log_index: log.log_index.unwrap(),
                    source,
                }
            })?;
            let announcement = SchemeId3::announcement_from_bytes(
                &decoded.data.stealthAddress.into_array(),
                &decoded.data.ephemeralPubKey,
                &decoded.data.metadata,
            );
            if let Some(announcement) = announcement {
                return Ok(Some(AnnouncementRecord::new(
                    announcement,
                    decoded.data.caller,
                    log.block_number.unwrap(),
                    log.block_hash.unwrap(),
                    log.transaction_hash.unwrap(),
                    log.log_index.unwrap(),
                )));
            }
            self.rejected_logs += 1;
        }
        Ok(None)
    }
}

fn block_ranges(from: u64, to: u64, size: NonZeroU64) -> impl Iterator<Item = RangeInclusive<u64>> {
    std::iter::successors((from <= to).then_some(from), move |start| {
        start.checked_add(size.get()).filter(|next| *next <= to)
    })
    .map(move |start| start..=start.saturating_add(size.get() - 1).min(to))
}

fn invalid_log(log: &Log, reason: &'static str) -> FetchError {
    FetchError::InvalidLog {
        block_number: log.block_number,
        log_index: log.log_index,
        reason,
    }
}

fn validate_logs(
    mut logs: Vec<Log>,
    announcer: Address,
    range: &RangeInclusive<u64>,
) -> Result<Vec<Log>, FetchError> {
    for log in &logs {
        if log.removed
            || log.block_number.is_none()
            || log.block_hash.is_none()
            || log.transaction_hash.is_none()
            || log.log_index.is_none()
        {
            return Err(invalid_log(log, "expected a mined, non-removed log"));
        }
        if !range.contains(&log.block_number.unwrap())
            || log.address() != announcer
            || log.topic0() != Some(&AnnouncementEvent::SIGNATURE_HASH)
            || log.topics().get(1) != Some(&B256::from(U256::from(SCHEME_ID)))
        {
            return Err(invalid_log(log, "log does not match the requested filter"));
        }
    }
    logs.sort_unstable_by_key(|log| (log.block_number, log.log_index));
    for pair in logs.windows(2) {
        if pair[0].block_number == pair[1].block_number {
            if pair[0].block_hash != pair[1].block_hash {
                return Err(invalid_log(
                    &pair[1],
                    "conflicting hashes for one finalized block",
                ));
            }
            if pair[0].log_index == pair[1].log_index {
                return Err(invalid_log(&pair[1], "duplicate log index"));
            }
        }
    }
    Ok(logs)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use alloy::{
        providers::{DynProvider, ProviderBuilder, RootProvider},
        rpc::json_rpc::ErrorPayload,
        rpc::types::Block,
        transports::{TransportResult, mock::Asserter},
    };
    use futures::{FutureExt, channel::oneshot, pin_mut};

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
            schemeId: U256::from(SCHEME_ID),
            stealthAddress: Address::repeat_byte(0xaa),
            caller: Address::repeat_byte(0xbb),
            ephemeralPubKey: vec![0x02; 33].into(),
            metadata: vec![0; 1089].into(),
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

    #[test]
    fn block_range_edges() {
        let size = NonZeroU64::new(2).unwrap();
        assert_eq!(
            block_ranges(u64::MAX - 2, u64::MAX, size).collect::<Vec<_>>(),
            vec![u64::MAX - 2..=u64::MAX - 1, u64::MAX..=u64::MAX]
        );
        assert_eq!(
            block_ranges(0, 4, size).collect::<Vec<_>>(),
            vec![0..=1, 2..=3, 4..=4]
        );
        assert_eq!(
            block_ranges(0, u64::MAX, NonZeroU64::MAX).collect::<Vec<_>>(),
            vec![0..=u64::MAX - 1, u64::MAX..=u64::MAX]
        );
        assert_eq!(block_ranges(2, 1, size).count(), 0);
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
            FetchConfig {
                blocks_per_request: NonZeroU64::new(2).unwrap(),
                ..Default::default()
            },
        )
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
        assert!(matches!(events.as_slice(), [
            FetchEvent::RangeFetched { range: first, rejected_logs: 0 },
            FetchEvent::RangeFetched { range: second, rejected_logs: 0 },
        ] if first == &(3..=4) && second == &(5..=5)));
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn sorts_logs_and_skips_bad_payloads() {
        let (provider, asserter) = mock(3);
        let mut logs = vec![log(2, 0), log(1, 0)];
        for (index, epk, metadata) in [
            (1, vec![0x02; 33], vec![0; 2000]),
            (2, vec![0; 33], vec![0; 1089]),
            (3, vec![0x02; 32], vec![0; 1089]),
            (4, vec![0x02; 33], vec![0; 1088]),
        ] {
            let mut malformed = log(1, index);
            let event = AnnouncementEvent {
                schemeId: U256::from(SCHEME_ID),
                stealthAddress: Address::ZERO,
                caller: Address::ZERO,
                ephemeralPubKey: epk.into(),
                metadata: metadata.into(),
            };
            malformed.inner.data = event.encode_log_data();
            logs.push(malformed);
        }
        asserter.push_success(&logs);
        let events = get_announcements(&provider, DEPLOYMENT, 1..=2, FetchConfig::default())
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        let positions: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                FetchEvent::Announcement(record) => {
                    Some((record.block_number(), record.log_index()))
                }
                FetchEvent::RangeFetched { .. } => None,
            })
            .collect();
        assert_eq!(positions, vec![(1, 0), (2, 0)]);
        assert!(matches!(events.last(),
            Some(FetchEvent::RangeFetched { range, rejected_logs: 4 }) if range == &(1..=2)));
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
            FetchConfig::default(),
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
    async fn rpc_range_fallback() {
        let (provider, asserter) = mock(1);
        asserter.push_failure_msg("range too large");
        asserter.push_success(&vec![log(0, 0)]);
        asserter.push_success(&vec![log(1, 0)]);
        let events = get_announcements(&provider, DEPLOYMENT, 0..=1, FetchConfig::default())
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(events.len(), 3);
        assert!(matches!(events.last(),
            Some(FetchEvent::RangeFetched { range, rejected_logs: 0 }) if range == &(0..=1)));
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn rpc_errors_stop_stream() {
        for (code, message) in [
            (-32005, "daily request count exceeded, request rate limited"),
            (429, "too many requests"),
            (-32602, "invalid filter"),
            (-32603, "request timed out"),
            (-32603, "unavailable"),
            (-32005, "unknown limit"),
            (-32601, "eth_getLogs unavailable: range too large"),
        ] {
            let (provider, asserter) = mock(1);
            asserter.push_failure(ErrorPayload {
                code,
                message: message.into(),
                data: None,
            });
            asserter.push_success(&Vec::<Log>::new());
            let events = get_announcements(&provider, DEPLOYMENT, 0..=1, FetchConfig::default());
            pin_mut!(events);
            let Err(FetchError::Rpc(error)) = events.try_next().await else {
                panic!("expected the original RPC error");
            };
            assert_eq!(error.as_error_resp().unwrap().code, code);
            assert_eq!(error.as_error_resp().unwrap().message, message);
            assert!(events.try_next().await.unwrap().is_none());
            assert_eq!(asserter.read_q().len(), 1, "must not query a smaller range");
        }

        let asserter = Asserter::new();
        asserter.push_success(&"0x1");
        asserter.push_success(&Option::<Block>::None);
        let provider = ProviderBuilder::new()
            .connect_mocked_client(asserter)
            .erased();
        let events = get_announcements(&provider, DEPLOYMENT, 0..=0, FetchConfig::default());
        pin_mut!(events);
        assert!(matches!(
            events.try_next().await,
            Err(FetchError::MissingFinalizedBlock)
        ));
        assert!(events.try_next().await.unwrap().is_none());
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
        let events = get_announcements(&provider, DEPLOYMENT, 0..=0, FetchConfig::default());
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
        asserter.push_success(&Vec::<Log>::new());
        let mut invalid = log(1, 0);
        invalid.transaction_hash = None;
        asserter.push_success(&vec![invalid]);
        asserter.push_success(&vec![log(2, 0)]);
        let events = get_announcements(
            &provider,
            DEPLOYMENT,
            0..=2,
            FetchConfig {
                blocks_per_request: NonZeroU64::new(1).unwrap(),
                ..Default::default()
            },
        );
        pin_mut!(events);
        assert!(matches!(events.try_next().await.unwrap(),
            Some(FetchEvent::RangeFetched { range, rejected_logs: 0 }) if range == (0..=0)));
        assert!(matches!(
            events.try_next().await,
            Err(FetchError::InvalidLog { .. })
        ));
        assert!(events.try_next().await.unwrap().is_none());
    }

    #[test]
    fn invalid_logs() {
        let valid = log(1, 0);
        let mut missing = valid.clone();
        missing.block_number = None;
        let mut removed = valid.clone();
        removed.removed = true;
        let mut foreign = valid.clone();
        foreign.inner.address = Address::ZERO;
        let mut other_hash = log(1, 1);
        other_hash.block_hash = Some(B256::ZERO);
        let mut other_scheme = valid.clone();
        other_scheme.topics_mut()[1] = B256::ZERO;
        for logs in [
            vec![missing],
            vec![removed],
            vec![foreign],
            vec![log(2, 0)],
            vec![other_scheme],
            vec![valid.clone(), valid.clone()],
            vec![valid.clone(), other_hash],
        ] {
            assert!(matches!(
                validate_logs(logs, ANNOUNCER, &(1..=1)),
                Err(FetchError::InvalidLog { .. })
            ));
        }
    }

    struct MockProvider {
        root: RootProvider,
        responses: Mutex<BTreeMap<u64, oneshot::Receiver<TransportResult<Vec<Log>>>>>,
        requested: Mutex<Vec<Filter>>,
    }

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        fn root(&self) -> &RootProvider {
            &self.root
        }

        async fn get_logs(&self, filter: &Filter) -> TransportResult<Vec<Log>> {
            self.requested.lock().unwrap().push(filter.clone());
            let from = filter
                .block_option
                .get_from_block()
                .unwrap()
                .as_number()
                .unwrap();
            let response = self.responses.lock().unwrap().remove(&from);
            match response {
                Some(response) => response.await.unwrap(),
                None => self.root.get_logs(filter).await,
            }
        }
    }

    #[tokio::test]
    async fn ordered_prefetch() {
        let (root, _) = mock(5);
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();
        let (third_tx, third_rx) = oneshot::channel();
        let provider = MockProvider {
            root: root.root().clone(),
            responses: Mutex::new(BTreeMap::from([
                (0, first_rx),
                (2, second_rx),
                (4, third_rx),
            ])),
            requested: Mutex::new(Vec::new()),
        };
        let mut events = Box::pin(get_announcements(
            &provider,
            DEPLOYMENT,
            0..=5,
            FetchConfig {
                blocks_per_request: NonZeroU64::new(2).unwrap(),
                ..Default::default()
            },
        ));
        assert!(provider.requested.lock().unwrap().is_empty());
        assert!(events.try_next().now_or_never().is_none());
        assert_eq!(provider.requested.lock().unwrap().len(), 2);
        second_tx.send(Ok(vec![])).unwrap();
        assert!(
            events.try_next().now_or_never().is_none(),
            "later range must wait for the gap"
        );
        assert_eq!(provider.requested.lock().unwrap().len(), 2);
        first_tx.send(Ok(vec![])).unwrap();
        assert!(matches!(events.try_next().await.unwrap(),
            Some(FetchEvent::RangeFetched { range, rejected_logs: 0 }) if range == (0..=1)));
        assert!(matches!(events.try_next().await.unwrap(),
            Some(FetchEvent::RangeFetched { range, rejected_logs: 0 }) if range == (2..=3)));
        assert!(events.try_next().now_or_never().is_none());
        assert_eq!(provider.requested.lock().unwrap().len(), 3);
        let requested = provider.requested.lock().unwrap();
        for (filter, (from, to)) in requested.iter().zip([(0, 1), (2, 3), (4, 5)]) {
            assert_eq!(
                filter.block_option.get_from_block().unwrap().as_number(),
                Some(from)
            );
            assert_eq!(
                filter.block_option.get_to_block().unwrap().as_number(),
                Some(to)
            );
            assert!(filter.matches(&log(from, 0).inner));
        }
        drop(requested);
        drop(events);
        assert!(
            third_tx.is_canceled(),
            "drop must release the pending request future"
        );
    }

    #[tokio::test]
    async fn adapts_to_rpc_limit() {
        let (root, asserter) = mock(1_999);
        let provider = MockProvider {
            root: root.root().clone(),
            responses: Mutex::new(BTreeMap::new()),
            requested: Mutex::new(Vec::new()),
        };
        for _ in 0..4 {
            asserter.push_failure(ErrorPayload {
                code: -32005,
                message: "query returned more than 10000 results".into(),
                data: None,
            });
        }
        let mut expected = vec![(0, 999), (0, 499), (0, 249), (0, 124)];
        let mut positions = Vec::new();
        for start in [0, 1_000] {
            for from in (start..start + 1_000).step_by(62) {
                expected.push((from, (from + 61).min(start + 999)));
                positions.push(from);
                asserter.push_success(&vec![log(from, 0)]);
            }
        }
        let events = get_announcements(
            &provider,
            DEPLOYMENT,
            0..=1_999,
            FetchConfig {
                prefetch: NonZeroUsize::new(1).unwrap(),
                ..Default::default()
            },
        )
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
        let actual: Vec<_> = provider
            .requested
            .lock()
            .unwrap()
            .iter()
            .map(|filter| {
                (
                    filter
                        .block_option
                        .get_from_block()
                        .unwrap()
                        .as_number()
                        .unwrap(),
                    filter
                        .block_option
                        .get_to_block()
                        .unwrap()
                        .as_number()
                        .unwrap(),
                )
            })
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 38);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    FetchEvent::Announcement(record) => Some(record.block_number()),
                    FetchEvent::RangeFetched { .. } => None,
                })
                .collect::<Vec<_>>(),
            positions
        );
        let completed: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                FetchEvent::RangeFetched {
                    range,
                    rejected_logs: 0,
                } => Some(range.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(completed, vec![0..=999, 1_000..=1_999]);
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn prefetch_shares_query_limit() {
        let (root, asserter) = mock(7);
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();
        let provider = MockProvider {
            root: root.root().clone(),
            responses: Mutex::new(BTreeMap::from([(0, first_rx), (4, second_rx)])),
            requested: Mutex::new(Vec::new()),
        };
        asserter.push_failure_msg("range too large");
        for block in 0..8 {
            asserter.push_success(&vec![log(block, 0)]);
        }
        let events = get_announcements(
            &provider,
            DEPLOYMENT,
            0..=7,
            FetchConfig {
                blocks_per_request: NonZeroU64::new(4).unwrap(),
                ..Default::default()
            },
        );
        pin_mut!(events);
        assert!(events.try_next().now_or_never().is_none());
        assert_eq!(provider.requested.lock().unwrap().len(), 2);
        let error = || {
            TransportError::ErrorResp(ErrorPayload::internal_error_message(
                "range too large".into(),
            ))
        };
        first_tx.send(Err(error())).unwrap();
        for block in 0..4 {
            let Some(FetchEvent::Announcement(record)) = events.try_next().await.unwrap() else {
                panic!("expected announcement");
            };
            assert_eq!(record.block_number(), block);
        }
        assert!(matches!(events.try_next().await.unwrap(),
            Some(FetchEvent::RangeFetched { range, .. }) if range == (0..=3)));
        second_tx.send(Err(error())).unwrap();
        for block in 4..8 {
            let Some(FetchEvent::Announcement(record)) = events.try_next().await.unwrap() else {
                panic!("expected announcement");
            };
            assert_eq!(record.block_number(), block);
        }
        assert!(matches!(events.try_next().await.unwrap(),
            Some(FetchEvent::RangeFetched { range, .. }) if range == (4..=7)));
        assert!(events.try_next().await.unwrap().is_none());
        let requests = provider.requested.lock().unwrap();
        assert_eq!(requests.len(), 11);
        for filter in &requests[3..] {
            assert_eq!(
                filter.block_option.get_from_block(),
                filter.block_option.get_to_block()
            );
        }
        assert!(asserter.read_q().is_empty());
    }

    #[test]
    fn log_limit_errors() {
        for (code, message, expected) in [
            (-32005, "query returned more than 10000 results", true),
            (-32005, "query timeout exceeded", true),
            (-32000, "Log response size exceeded", true),
            (-32602, "eth_getLogs is limited to a 100 blocks range", true),
            (-32602, "block range is too wide", true),
            (-32602, "block range limit exceeded", true),
            (429, "query timeout exceeded", false),
            (-32005, "exceeded project rate limit", false),
            (-32602, "invalid block range params", false),
        ] {
            let error = TransportError::ErrorResp(ErrorPayload {
                code,
                message: message.into(),
                data: None,
            });
            assert_eq!(is_log_limit(&error), expected, "{code}: {message}");
        }
        assert!(!is_log_limit(&TransportError::local_usage_str(
            "range too large"
        )));
    }

    #[tokio::test]
    async fn failed_page_does_not_complete_batch() {
        let (provider, asserter) = mock(3);
        asserter.push_failure_msg("range too large");
        asserter.push_success(&vec![log(0, 0)]);
        asserter.push_failure_msg("unavailable");
        let events = get_announcements(&provider, DEPLOYMENT, 0..=3, FetchConfig::default());
        pin_mut!(events);
        assert!(matches!(events.try_next().await, Err(FetchError::Rpc(_))));
        assert!(events.try_next().await.unwrap().is_none());
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn rejects_logs_outside_page() {
        let (provider, asserter) = mock(3);
        asserter.push_failure_msg("range too large");
        asserter.push_success(&vec![log(3, 0)]);
        let events = get_announcements(&provider, DEPLOYMENT, 0..=3, FetchConfig::default());
        pin_mut!(events);
        assert!(matches!(
            events.try_next().await,
            Err(FetchError::InvalidLog { .. })
        ));
        assert!(events.try_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn single_block_limit_stops_stream() {
        let (provider, asserter) = mock(0);
        asserter.push_failure_msg("range too large");
        asserter.push_success(&Vec::<Log>::new());
        let events = get_announcements(&provider, DEPLOYMENT, 0..=0, FetchConfig::default());
        pin_mut!(events);
        assert!(matches!(events.try_next().await, Err(FetchError::Rpc(_))));
        assert!(events.try_next().await.unwrap().is_none());
        assert_eq!(asserter.read_q().len(), 1);
    }

    #[tokio::test]
    async fn adapts_at_last_block() {
        let (provider, asserter) = mock(u64::MAX);
        asserter.push_failure_msg("range too large");
        for block in [u64::MAX - 4, u64::MAX - 2, u64::MAX] {
            asserter.push_success(&vec![log(block, 0)]);
        }
        let events = get_announcements(
            &provider,
            DEPLOYMENT,
            u64::MAX - 4..=u64::MAX,
            FetchConfig {
                blocks_per_request: NonZeroU64::new(5).unwrap(),
                ..Default::default()
            },
        )
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
        assert_eq!(events.len(), 4);
        assert!(matches!(events.last(),
            Some(FetchEvent::RangeFetched { range, rejected_logs: 0 })
                if range == &(u64::MAX - 4..=u64::MAX)));
        assert!(asserter.read_q().is_empty());
    }
}
