use std::{
    collections::{BTreeMap, HashMap},
    ops::RangeInclusive,
    sync::Arc,
};

use kohaku_kv_store::{Store, backend::StoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::info;

use crate::{
    contracts::Deployment,
    scheme3::SCHEME_ID,
    syncer::{AnnouncementRecord, AnnouncementSyncer, SyncerError},
};

const STATE_KEY: &[u8] = b"state";
const DEFAULT_REORG_DEPTH: u64 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexerConfig {
    pub reorg_depth: u64,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            reorg_depth: DEFAULT_REORG_DEPTH,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncReport {
    pub from_block: u64,
    pub to_block: u64,
    pub fetched: usize,
    pub stored: usize,
    pub rejected_logs: usize,
}

#[derive(Debug, Error)]
pub enum IndexerError {
    #[error(transparent)]
    Syncer(#[from] SyncerError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("announcement index could not be encoded or decoded")]
    CorruptStore(#[source] postcard::Error),
    #[error("stored announcement block {0} is missing")]
    MissingBlock(u64),
    #[error("stored announcement block {0} is invalid")]
    InvalidBlock(u64),
    #[error("announcement source returned block {block} outside {from}..={to}")]
    BlockOutOfRange { block: u64, from: u64, to: u64 },
    #[error("announcement source returned conflicting data at block {block}, log {log_index}")]
    ConflictingLog { block: u64, log_index: u64 },
    #[error("announcement source returned more than one hash for block {0}")]
    ConflictingBlockHash(u64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IndexState {
    cursor: u64,
    blocks: Vec<BlockEntry>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct BlockEntry {
    number: u64,
    count: u64,
}

#[derive(Clone)]
pub struct AnnouncementIndexer {
    store: Store,
    syncer: AnnouncementSyncer,
    start_block: u64,
    config: IndexerConfig,
    sync_lock: Arc<Mutex<()>>,
}

impl AnnouncementIndexer {
    #[must_use]
    pub fn new(
        store: &Store,
        syncer: AnnouncementSyncer,
        deployment: Deployment,
        config: IndexerConfig,
    ) -> Self {
        let store = store.scope(format!(
            "kohaku-stealth/index/v2/{}/{:x}/{SCHEME_ID}",
            deployment.chain_id, deployment.announcer
        ));
        Self {
            store,
            syncer,
            start_block: deployment.start_block,
            config,
            sync_lock: Arc::new(Mutex::new(())),
        }
    }

    #[must_use]
    pub const fn with_config(mut self, config: IndexerConfig) -> Self {
        self.config = config;
        self
    }

    /// Returns validated announcements currently held in the local index.
    ///
    /// # Errors
    ///
    /// Returns an error when storage cannot be read or contains invalid data.
    pub async fn announcements(&self) -> Result<Vec<AnnouncementRecord>, IndexerError> {
        load_announcements(&self.store, None).await
    }

    /// Returns validated announcements in an inclusive block range.
    ///
    /// # Errors
    ///
    /// Returns an error when storage cannot be read or contains invalid data.
    pub async fn announcements_in(
        &self,
        range: RangeInclusive<u64>,
    ) -> Result<Vec<AnnouncementRecord>, IndexerError> {
        load_announcements(&self.store, Some(&range)).await
    }

    /// Synchronizes the local announcement index with its configured source.
    ///
    /// The last `reorg_depth` blocks are replaced on every sync.
    ///
    /// # Errors
    ///
    /// Returns an error when the source or local storage fails.
    pub async fn sync(&self) -> Result<SyncReport, IndexerError> {
        let _guard = self.sync_lock.lock().await;
        let latest = self.syncer.latest_block().await?;
        let current = load_state(&self.store).await?;
        let from = current
            .as_ref()
            .map(|state| state.cursor)
            .map_or(self.start_block, |saved| {
                saved.min(latest).saturating_sub(self.config.reorg_depth)
            })
            .max(self.start_block);

        let mut stable_blocks = current.map_or_else(Vec::new, |state| state.blocks);
        if from > latest {
            stable_blocks.retain(|entry| entry.number <= latest);
            let state = IndexState {
                cursor: latest,
                blocks: stable_blocks,
            };
            save_blocks_and_state(&self.store, &BTreeMap::new(), &state).await?;
            let stored = stored_count(&state.blocks)?;
            info!(
                from_block = from,
                to_block = latest,
                stored,
                "announcement index synchronized before deployment"
            );
            return Ok(SyncReport {
                from_block: from,
                to_block: latest,
                fetched: 0,
                stored,
                rejected_logs: 0,
            });
        }

        let fetched = self.syncer.fetch_announcements(from..=latest).await?;
        let fetched_count = fetched.announcements.len();
        let new_blocks = group_by_block(fetched.announcements, from, latest)?;
        stable_blocks.retain(|entry| entry.number < from);
        stable_blocks.extend(new_blocks.iter().map(|(&number, records)| BlockEntry {
            number,
            count: records.len() as u64,
        }));
        let state = IndexState {
            cursor: latest,
            blocks: stable_blocks,
        };
        save_blocks_and_state(&self.store, &new_blocks, &state).await?;
        let stored = stored_count(&state.blocks)?;
        let report = SyncReport {
            from_block: from,
            to_block: latest,
            fetched: fetched_count,
            stored,
            rejected_logs: fetched.rejected_logs,
        };
        info!(
            from_block = report.from_block,
            to_block = report.to_block,
            fetched = report.fetched,
            stored = report.stored,
            rejected_logs = report.rejected_logs,
            "announcement index synchronized"
        );
        Ok(report)
    }
}

async fn load_state(store: &Store) -> Result<Option<IndexState>, IndexerError> {
    let Some(bytes) = store.get(STATE_KEY).await? else {
        return Ok(None);
    };
    let state: IndexState = postcard::from_bytes(&bytes).map_err(IndexerError::CorruptStore)?;
    if state
        .blocks
        .windows(2)
        .any(|pair| pair[0].number >= pair[1].number)
    {
        return Err(IndexerError::InvalidBlock(state.cursor));
    }
    Ok(Some(state))
}

async fn load_announcements(
    store: &Store,
    range: Option<&RangeInclusive<u64>>,
) -> Result<Vec<AnnouncementRecord>, IndexerError> {
    let Some(state) = load_state(store).await? else {
        return Ok(Vec::new());
    };
    let blocks: Vec<BlockEntry> = state
        .blocks
        .into_iter()
        .filter(|entry| range.is_none_or(|range| range.contains(&entry.number)))
        .collect();
    let keys: Vec<Vec<u8>> = blocks.iter().map(|entry| block_key(entry.number)).collect();
    let values = store.get_batch(&keys).await?;
    let mut announcements = Vec::with_capacity(stored_count(&blocks)?);
    for (entry, value) in blocks.iter().zip(values) {
        let bytes = value.ok_or(IndexerError::MissingBlock(entry.number))?;
        let records: Vec<AnnouncementRecord> =
            postcard::from_bytes(&bytes).map_err(IndexerError::CorruptStore)?;
        if u64::try_from(records.len()).ok() != Some(entry.count)
            || records
                .iter()
                .any(|record| record.block_number() != entry.number || !record.is_valid())
        {
            return Err(IndexerError::InvalidBlock(entry.number));
        }
        announcements.extend(records);
    }
    Ok(announcements)
}

fn group_by_block(
    announcements: Vec<AnnouncementRecord>,
    from: u64,
    to: u64,
) -> Result<BTreeMap<u64, Vec<AnnouncementRecord>>, IndexerError> {
    let mut blocks = BTreeMap::<u64, Vec<AnnouncementRecord>>::new();
    for announcement in announcements {
        let block = announcement.block_number();
        if !(from..=to).contains(&block) {
            return Err(IndexerError::BlockOutOfRange { block, from, to });
        }
        if !announcement.is_valid() {
            return Err(IndexerError::InvalidBlock(block));
        }
        blocks.entry(block).or_default().push(announcement);
    }

    for (&block, records) in &mut blocks {
        if let Some(block_hash) = records.first().map(AnnouncementRecord::block_hash)
            && records
                .iter()
                .any(|record| record.block_hash() != block_hash)
        {
            return Err(IndexerError::ConflictingBlockHash(block));
        }
        let mut unique = Vec::with_capacity(records.len());
        let mut positions = HashMap::with_capacity(records.len());
        for record in records.drain(..) {
            let id = record.id();
            if let Some(&position) = positions.get(&id) {
                if unique[position] != record {
                    return Err(IndexerError::ConflictingLog {
                        block,
                        log_index: record.log_index(),
                    });
                }
            } else {
                positions.insert(id, unique.len());
                unique.push(record);
            }
        }
        unique.sort_unstable_by_key(AnnouncementRecord::log_index);
        *records = unique;
    }
    Ok(blocks)
}

fn stored_count(blocks: &[BlockEntry]) -> Result<usize, IndexerError> {
    blocks.iter().try_fold(0usize, |total, entry| {
        usize::try_from(entry.count)
            .ok()
            .and_then(|count| total.checked_add(count))
            .ok_or(IndexerError::InvalidBlock(entry.number))
    })
}

fn block_key(block_number: u64) -> Vec<u8> {
    format!("block/{block_number:016x}").into_bytes()
}

async fn save_blocks_and_state(
    store: &Store,
    blocks: &BTreeMap<u64, Vec<AnnouncementRecord>>,
    state: &IndexState,
) -> Result<(), IndexerError> {
    let mut items = Vec::with_capacity(blocks.len() + 1);
    for (&block_number, records) in blocks {
        items.push((
            block_key(block_number),
            postcard::to_allocvec(records).map_err(IndexerError::CorruptStore)?,
        ));
    }
    items.push((
        STATE_KEY.to_vec(),
        postcard::to_allocvec(state).map_err(IndexerError::CorruptStore)?,
    ));
    store.put_batch(items).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{ops::RangeInclusive, sync::Mutex as StdMutex};

    use alloy::primitives::{Address, B256};

    use super::*;
    use crate::{
        scheme3::{Scheme3Account, generate_stealth_address_with_seed},
        syncer::{AnnouncementSyncerBackend, FetchResult},
    };

    #[derive(Clone)]
    struct MockSyncer {
        state: Arc<StdMutex<MockState>>,
    }

    #[derive(Clone)]
    struct MockState {
        latest: u64,
        records: Vec<AnnouncementRecord>,
    }

    #[async_trait::async_trait]
    impl AnnouncementSyncerBackend for MockSyncer {
        async fn latest_block(&self) -> Result<u64, SyncerError> {
            Ok(self.state.lock().unwrap().latest)
        }

        async fn fetch_announcements(
            &self,
            range: RangeInclusive<u64>,
        ) -> Result<FetchResult, SyncerError> {
            let state = self.state.lock().unwrap();
            Ok(FetchResult {
                announcements: state
                    .records
                    .iter()
                    .filter(|record| range.contains(&record.block_number()))
                    .cloned()
                    .collect(),
                rejected_logs: 0,
            })
        }
    }

    fn account() -> Scheme3Account {
        let mut seed = [0u8; 128];
        seed[..32].fill(0x11);
        seed[32..64].fill(0x22);
        seed[64..].fill(0x33);
        Scheme3Account::from_seed(&seed).unwrap()
    }

    fn record(seed: u8, block_hash: u8, transaction_hash: u8) -> AnnouncementRecord {
        let account = account();
        let generated =
            generate_stealth_address_with_seed(account.meta_address(), &[seed; 64]).unwrap();
        AnnouncementRecord::new(
            &generated.announcement,
            Address::repeat_byte(0x44),
            10,
            B256::repeat_byte(block_hash),
            B256::repeat_byte(transaction_hash),
            0,
        )
    }

    #[tokio::test]
    async fn sync_deduplicates_and_replaces_the_reorg_window() {
        let first = record(0x42, 0xaa, 0x11);
        let state = Arc::new(StdMutex::new(MockState {
            latest: 10,
            records: vec![first.clone(), first],
        }));
        let syncer = MockSyncer {
            state: Arc::clone(&state),
        };
        let deployment = Deployment {
            chain_id: 1,
            announcer: Address::repeat_byte(0x55),
            registry: Address::repeat_byte(0x66),
            start_block: 0,
        };
        let indexer = AnnouncementIndexer::new(
            &Store::create(),
            syncer.into(),
            deployment,
            IndexerConfig::default(),
        );

        let first_report = indexer.sync().await.unwrap();
        assert_eq!(first_report.fetched, 2);
        assert_eq!(first_report.stored, 1);

        let replacement = record(0x43, 0xbb, 0x22);
        state.lock().unwrap().records = vec![replacement.clone()];
        let second_report = indexer.sync().await.unwrap();
        assert_eq!(second_report.stored, 1);
        assert_eq!(indexer.announcements().await.unwrap(), vec![replacement]);
    }
}
