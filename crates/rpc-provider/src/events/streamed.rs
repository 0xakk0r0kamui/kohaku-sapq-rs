//! Adaptive, concurrent `eth_getLogs` queries over large block ranges.
//!
//! Similar to alloy's [`alloy::contract::ChunkedEvent`] but with exponential backoff.

use std::{
    marker::PhantomData,
    ops::Range,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use alloy::{
    network::{Ethereum, Network},
    providers::Provider,
    rpc::types::{BlockNumberOrTag, Filter, FilterBlockOption, Log},
    transports::{
        RpcError, TransportError, TransportErrorKind, TransportResult,
        layers::{RateLimitRetryPolicy, RetryPolicy},
    },
};
use futures::{Stream, StreamExt, TryStreamExt, stream};
use tokio::time::{Instant, sleep_until};
use tracing::debug;

/// Streamed event query.
///
/// Queries the [`Filter`]'s block range in chunks, up to `max_concurrent` at once, yielding them
/// in block order.
///
/// Uses exponential backoff to handle rate limits and other errors:
/// - Rate limits: Retries the same range after a request pause. Doubles the pause on each error up
///   to `max_delay` and resets the pause on a successful request.
/// - Other errors: Halves the range and retries each half.
/// - Single block failures: Returns the error, ending the stream.
///
/// ```no_run
/// # async fn example(
/// #     provider: impl alloy::providers::Provider + Clone,
/// # ) -> alloy::transports::TransportResult<()> {
/// use alloy::rpc::types::Filter;
/// use kohaku_rpc_provider::{StreamedEvent, StreamedEventConfig};
///
/// let filter = Filter::new().from_block(9_000_000).to_block(9_100_000);
/// let logs = StreamedEvent::new(provider, filter)
///     .config(StreamedEventConfig {
///         chunk_size: 10_000,
///         max_concurrent: 4,
///         ..Default::default()
///     })
///     .query()
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct StreamedEvent<P: Provider<N>, N: Network = Ethereum> {
    provider: P,
    filter: Filter,
    config: StreamedEventConfig,
    _network: PhantomData<N>,
}

/// Tuning options for a [`StreamedEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamedEventConfig {
    /// The initial chunk size in blocks (default: 1,000).
    ///
    /// Clamped to 1..
    pub chunk_size: u64,
    /// The maximum number of concurrent chunk requests (default: 5).
    ///
    /// Clamped to 1..
    pub max_concurrent: usize,
    /// The rate-limit pause lower bound (default: 250ms).
    pub base_delay: Duration,
    /// The rate-limit pause upper bound (default: 30s).
    pub max_delay: Duration,
    /// How many consecutive rate limits a single request tolerates (default: 10).
    pub max_retries: u32,
}

/// Adaptive state shared by every request in one stream.
struct State {
    /// Block count for chunks not yet started.
    size: u64,
    /// Pause applied on the next rate limit.
    delay: Duration,
    /// While paused, the next free send slot. Each waiting request claims it and pushes it back.
    paused_until: Option<Instant>,
}

impl<P: Provider<N>, N: Network> StreamedEvent<P, N> {
    /// Creates a new query for `filter`.
    #[must_use]
    pub fn new(provider: P, filter: Filter) -> Self {
        Self {
            provider,
            filter,
            config: StreamedEventConfig::default(),
            _network: PhantomData,
        }
    }

    /// Replaces all tuning options at once.
    #[must_use]
    pub fn config(mut self, config: StreamedEventConfig) -> Self {
        self.config = config;
        self
    }
}

impl<P: Provider<N> + Clone, N: Network> StreamedEvent<P, N> {
    /// Queries the blockchain for all matching logs, in block order.
    pub async fn query(&self) -> TransportResult<Vec<Log>> {
        self.stream().try_collect().await
    }

    /// Streams matching logs from the blockchain, in block order.
    ///
    /// Ends the stream upon encountering a single-block failure.
    pub fn stream(&self) -> impl Stream<Item = TransportResult<Log>> + use<P, N> {
        let this = self.clone();
        let logs = Box::pin(
            stream::once(async move {
                let range = this.resolve_block_range().await?;
                debug!(?range, "resolved block range");
                Ok::<_, TransportError>(this.chunk_stream(range))
            })
            .try_flatten()
            .map_ok(|logs| stream::iter(logs.into_iter().map(Ok)))
            .try_flatten(),
        );

        //? Dropping `logs` after an error cancels any chunks still in flight.
        stream::unfold(Some(logs), |logs| async move {
            let mut logs = logs?;
            let item = logs.next().await?;
            let logs = item.is_ok().then_some(logs);
            Some((item, logs))
        })
    }

    /// Streams the logs of `range` chunk by chunk, querying up to `max_concurrent` chunks at once.
    fn chunk_stream(self, range: Range<u64>) -> impl Stream<Item = TransportResult<Vec<Log>>> {
        let max_concurrent = self.config.max_concurrent.max(1);
        let state = Arc::new(Mutex::new(State {
            size: self.config.chunk_size.max(1),
            delay: self.config.base_delay,
            paused_until: None,
        }));

        stream::iter(chunk_ranges(range, state.clone()))
            .map(move |range| {
                let this = self.clone();
                let state = state.clone();
                async move {
                    let range = range?;
                    let logs = this.query_chunk(&state, range.clone()).await?;
                    debug!(?range, logs = logs.len(), "fetched chunk");
                    Ok(logs)
                }
            })
            .buffered(max_concurrent)
    }

    /// Resolves the filter's inclusive block bounds into an exclusive range.
    async fn resolve_block_range(&self) -> TransportResult<Range<u64>> {
        let FilterBlockOption::Range {
            from_block,
            to_block,
        } = self.filter.block_option
        else {
            return Err(RpcError::local_usage_str(
                "chunked queries require a block range filter, not a block hash filter",
            ));
        };

        let from = self
            .resolve_block_tag(from_block.unwrap_or(BlockNumberOrTag::Earliest))
            .await?;
        let to = self
            .resolve_block_tag(to_block.unwrap_or(BlockNumberOrTag::Latest))
            .await?;

        //? Both filter bounds are inclusive, the returned range's end is exclusive.
        Ok(from..to.saturating_add(1))
    }

    /// Resolves `tag` to a block number. Tags other than `earliest` (including `safe` and
    /// `finalized`) resolve to the latest block.
    async fn resolve_block_tag(&self, tag: BlockNumberOrTag) -> TransportResult<u64> {
        match tag {
            BlockNumberOrTag::Number(n) => Ok(n),
            BlockNumberOrTag::Earliest => Ok(0),
            _ => self.provider.get_block_number().await,
        }
    }

    /// Queries `chunk`, halving any sub-range that fails for a reason other than rate limiting.
    async fn query_chunk(
        &self,
        state: &Mutex<State>,
        chunk: Range<u64>,
    ) -> TransportResult<Vec<Log>> {
        let mut logs = Vec::new();
        //? Popped from the back, so the left half is always queried first.
        let mut pending = vec![chunk];

        while let Some(range) = pending.pop() {
            let err = match self.query_range(state, range.clone()).await {
                Ok(found) => {
                    logs.extend(found);
                    continue;
                }
                Err(err) => err,
            };

            if is_rate_limited(&err) || range.end - range.start <= 1 {
                return Err(err);
            }

            //? A poisoned lock error lands here too, but is returned by `lock` before any split.
            let mid = range.start + (range.end - range.start) / 2;
            lock(state)?.shrink(mid - range.start);
            debug!(%err, ?range, "eth_getLogs failed, splitting range");
            pending.push(mid..range.end);
            pending.push(range.start..mid);
        }

        Ok(logs)
    }

    /// Queries `range`, waiting out the shared pause and retrying while rate limited.
    async fn query_range(
        &self,
        state: &Mutex<State>,
        range: Range<u64>,
    ) -> TransportResult<Vec<Log>> {
        //? `eth_getLogs` treats both block bounds as inclusive
        let filter = self
            .filter
            .clone()
            .from_block(range.start)
            .to_block(range.end - 1);
        let max_concurrent = self.config.max_concurrent.max(1);
        let stagger = self.config.base_delay / u32::try_from(max_concurrent).unwrap_or(u32::MAX);
        let mut retries = 0;

        loop {
            let slot = lock(state)?.claim_slot(stagger);
            if let Some(slot) = slot {
                sleep_until(slot).await;
            }

            let err = match self.provider.get_logs(&filter).await {
                Ok(logs) => {
                    lock(state)?.succeeded(self.config.base_delay);
                    return Ok(logs);
                }
                Err(err) => err,
            };

            if !is_rate_limited(&err) || retries >= self.config.max_retries {
                return Err(err);
            }

            retries += 1;
            debug!(%err, ?range, retries, "eth_getLogs rate limited, pausing");
            let hint = RateLimitRetryPolicy::default().backoff_hint(&err);
            lock(state)?.pause(hint, self.config.max_delay);
        }
    }
}

impl State {
    fn shrink(&mut self, size: u64) {
        self.size = self.size.min(size).max(1);
    }

    fn claim_slot(&mut self, stagger: Duration) -> Option<Instant> {
        let slot = self.paused_until?;
        if slot <= Instant::now() {
            self.paused_until = None;
            return None;
        }

        self.paused_until = Some(slot + stagger);
        Some(slot)
    }

    fn succeeded(&mut self, base_delay: Duration) {
        if self.paused_until.is_none() {
            self.delay = base_delay;
        }
    }

    fn pause(&mut self, hint: Option<Duration>, max_delay: Duration) {
        let until = Instant::now() + hint.unwrap_or(self.delay);
        self.paused_until = Some(self.paused_until.map_or(until, |prev| prev.max(until)));
        self.delay = (self.delay * 2).min(max_delay);
    }
}

impl Default for StreamedEventConfig {
    fn default() -> Self {
        Self {
            chunk_size: 1_000,
            max_concurrent: 5,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(30),
            max_retries: 10,
        }
    }
}

/// Lazily splits `range` into chunks.
///
/// Reads the current size as each chunk is started. Ends after yielding an error.
fn chunk_ranges(
    range: Range<u64>,
    state: Arc<Mutex<State>>,
) -> impl Iterator<Item = TransportResult<Range<u64>>> {
    let mut start = range.start;
    std::iter::from_fn(move || {
        if start >= range.end {
            return None;
        }

        let size = match lock(&state) {
            Ok(state) => state.size,
            Err(err) => {
                start = range.end;
                return Some(Err(err));
            }
        };

        let end = start.saturating_add(size).min(range.end);
        let chunk = start..end;
        start = end;
        Some(Ok(chunk))
    })
}

fn is_rate_limited(err: &TransportError) -> bool {
    RateLimitRetryPolicy::default().should_retry(err)
}

fn lock(state: &Mutex<State>) -> TransportResult<MutexGuard<'_, State>> {
    state
        .lock()
        .map_err(|_| TransportErrorKind::custom_str("StreamedEvent state lock poisoned"))
}

#[cfg(test)]
mod tests {
    use alloy::{
        providers::ProviderBuilder, rpc::json_rpc::ErrorPayload, transports::mock::Asserter,
    };

    use super::*;

    #[tokio::test]
    async fn failed_range_is_split_and_kept_in_order() {
        let asserter = Asserter::new();
        asserter.push_failure_msg("query returned more than 10000 results");
        asserter.push_success(&vec![log_at(1)]);
        asserter.push_success(&vec![log_at(3)]);
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        let logs =
            StreamedEvent::<_, Ethereum>::new(provider, Filter::new().from_block(0).to_block(4))
                .config(StreamedEventConfig {
                    max_concurrent: 1,
                    ..Default::default()
                })
                .query()
                .await
                .unwrap();

        assert_eq!(blocks(&logs), vec![1, 3]);
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limited_range_is_retried_after_pause() {
        let asserter = Asserter::new();
        asserter.push_failure(rate_limit());
        asserter.push_success(&vec![log_at(2)]);
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        let start = Instant::now();
        let logs =
            StreamedEvent::<_, Ethereum>::new(provider, Filter::new().from_block(0).to_block(4))
                .config(StreamedEventConfig {
                    base_delay: Duration::from_secs(1),
                    max_delay: Duration::from_secs(1),
                    ..Default::default()
                })
                .query()
                .await
                .unwrap();

        assert_eq!(blocks(&logs), vec![2]);
        assert!(start.elapsed() >= Duration::from_secs(1));
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_fails_after_max_retries() {
        let asserter = Asserter::new();
        asserter.push_failure(rate_limit());
        asserter.push_failure(rate_limit());
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        let result =
            StreamedEvent::<_, Ethereum>::new(provider, Filter::new().from_block(0).to_block(4))
                .config(StreamedEventConfig {
                    max_retries: 1,
                    ..Default::default()
                })
                .query()
                .await;

        assert!(result.is_err());
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn failed_single_block_ends_stream() {
        let asserter = Asserter::new();
        asserter.push_failure_msg("internal error");
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());

        //? Two one-block chunks: the second is never requested once the first fails.
        let items: Vec<_> =
            StreamedEvent::<_, Ethereum>::new(provider, Filter::new().from_block(0).to_block(1))
                .config(StreamedEventConfig {
                    chunk_size: 1,
                    max_concurrent: 1,
                    ..Default::default()
                })
                .stream()
                .collect()
                .await;

        assert_eq!(items.len(), 1);
        assert!(items[0].is_err());
    }

    fn rate_limit() -> ErrorPayload {
        ErrorPayload {
            code: 429,
            message: "rate limited".into(),
            data: None,
        }
    }

    fn log_at(block: u64) -> Log {
        Log {
            block_number: Some(block),
            ..Default::default()
        }
    }

    fn blocks(logs: &[Log]) -> Vec<u64> {
        logs.iter().filter_map(|log| log.block_number).collect()
    }
}
