use std::{io::Read, ops::Range};

use alloy::{
    primitives::{Address, B256},
    sol_types::SolEvent,
};
use kohaku_tornadocash::event::{DepositEvent, SyncEvent, WithdrawalEvent};
use sha2::{Digest as _, Sha256};
use tracing::warn;

use super::{
    SagaSyncError,
    manifest::{ChunkRef, deserialize_hex_u64},
};
use crate::abis::Tornado;

/// One line of a saga-sync chunk file.
///
/// <https://github.com/fatlabsxyz/saga-sync/blob/master/SPEC.md#32-chunk>
#[expect(dead_code)]
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SagaEvent {
    contract_address: Address,
    event_topic: B256,
    topics: Vec<B256>,
    data: String,
    #[serde(deserialize_with = "deserialize_hex_u64")]
    block_number: u64,
    #[serde(deserialize_with = "deserialize_hex_u64")]
    log_index: u64,
}

impl SagaEvent {
    fn decode(&self) -> Result<Option<SyncEvent>, SagaSyncError> {
        let data = hex::decode(self.data.strip_prefix("0x").unwrap_or(&self.data))?;

        match self.topics.first() {
            Some(&Tornado::Deposit::SIGNATURE_HASH) => {
                let decoded = Tornado::Deposit::decode_raw_log(self.topics.iter().copied(), &data)?;
                Ok(Some(SyncEvent::Deposit(DepositEvent {
                    commitment: decoded.commitment.try_into()?,
                    leaf_index: decoded.leafIndex,
                    block_number: self.block_number,
                })))
            }
            Some(&Tornado::Withdrawal::SIGNATURE_HASH) => {
                let decoded =
                    Tornado::Withdrawal::decode_raw_log(self.topics.iter().copied(), &data)?;
                Ok(Some(SyncEvent::Withdrawal(WithdrawalEvent {
                    to: decoded.to,
                    nullifier_hash: decoded.nullifierHash.try_into()?,
                    relayer: decoded.relayer,
                    fee: decoded.fee,
                    block_number: self.block_number,
                })))
            }
            _ => Ok(None),
        }
    }
}

/// Gzip-decompresses a chunk's bytes and verifies them against the manifest's sha256 digest.
///
/// # Errors
/// Returns an error if decompression fails or the digest doesn't match.
pub fn verify_and_decompress_chunk(
    gz_bytes: &[u8],
    chunk: &ChunkRef,
) -> Result<Vec<u8>, SagaSyncError> {
    let mut decoder = flate2::read::GzDecoder::new(gz_bytes);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;

    let digest = Sha256::digest(&decompressed);
    let actual = format!("0x{}", hex::encode(digest));
    let expected = chunk.digest.data.to_lowercase();
    if actual != expected {
        return Err(SagaSyncError::DigestMismatch { expected, actual });
    }

    Ok(decompressed)
}

/// Parses a chunk's JSONL events and decodes them into [`SyncEvent`]s that fall within the
/// caller-requested `range`.
///
/// # Errors
/// Returns an error if a line fails to parse, events are out of order, or an event's block
/// number falls outside the chunk's own declared range.
pub fn parse_chunk_events(
    decompressed: &[u8],
    chunk: &ChunkRef,
    range: Range<u64>,
) -> Result<Vec<SyncEvent>, SagaSyncError> {
    let text = std::str::from_utf8(decompressed)?;

    let mut events = Vec::new();
    let mut prev: Option<(u64, u64)> = None;

    for (index, line) in text.lines().filter(|l| !l.is_empty()).enumerate() {
        let ev: SagaEvent = serde_json::from_str(line)?;
        let key = (ev.block_number, ev.log_index);

        if let Some(prev_key) = prev
            && key <= prev_key
        {
            return Err(SagaSyncError::OutOfOrder { index });
        }
        prev = Some(key);

        if ev.block_number < chunk.from_block || ev.block_number >= chunk.to_block {
            return Err(SagaSyncError::OutOfRange {
                block: ev.block_number,
                from: chunk.from_block,
                to: chunk.to_block,
            });
        }

        if !range.contains(&ev.block_number) {
            continue;
        }

        if let Some(sync_event) = ev.decode()? {
            events.push(sync_event);
        } else {
            warn!(
                topic = ?ev.topics.first(),
                block = ev.block_number,
                "Skipping saga-sync event with unrecognized topic"
            );
        }
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::{Compression, write::GzEncoder};

    use super::*;
    use crate::saga::manifest::Digest;

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn chunk_ref(from_block: u64, to_block: u64, digest: &[u8]) -> ChunkRef {
        ChunkRef {
            from_block,
            to_block,
            file: "chunk.jsonl.gz".into(),
            size: 0,
            digest: Digest {
                kind: "sha256".into(),
                data: format!("0x{}", hex::encode(digest)),
            },
        }
    }

    fn fixture_line(block_number: &str, log_index: &str) -> String {
        let topic = B256::repeat_byte(0x11);
        serde_json::json!({
            "contractAddress": "0x12d66f87a04a9e220743712ce6d9bb1b5616b8fc",
            "eventTopic": format!("{topic:?}"),
            "topics": [format!("{topic:?}")],
            "data": "0x",
            "blockNumber": block_number,
            "logIndex": log_index,
        })
        .to_string()
    }

    #[test]
    fn verifies_matching_digest() {
        let bytes = b"hello saga-sync";
        let digest = Sha256::digest(bytes);
        let chunk = chunk_ref(0, 100, &digest);

        let decompressed = verify_and_decompress_chunk(&gzip(bytes), &chunk).unwrap();
        assert_eq!(decompressed, bytes);
    }

    #[test]
    fn rejects_mismatched_digest() {
        let bytes = b"hello saga-sync";
        let wrong_digest = Sha256::digest(b"tampered");
        let chunk = chunk_ref(0, 100, &wrong_digest);

        let err = verify_and_decompress_chunk(&gzip(bytes), &chunk).unwrap_err();
        assert!(matches!(err, SagaSyncError::DigestMismatch { .. }));
    }

    #[test]
    fn rejects_out_of_order_events() {
        let chunk = chunk_ref(0, 100, &[]);
        let body = format!(
            "{}\n{}\n",
            fixture_line("0x5", "0x1"),
            fixture_line("0x5", "0x1")
        );

        let result = parse_chunk_events(body.as_bytes(), &chunk, 0..100);
        assert!(matches!(result, Err(SagaSyncError::OutOfOrder { .. })));
    }

    #[test]
    fn rejects_event_outside_chunk_range() {
        let chunk = chunk_ref(0, 10, &[]);
        let body = format!("{}\n", fixture_line("0x64", "0x0"));

        let result = parse_chunk_events(body.as_bytes(), &chunk, 0..1000);
        assert!(matches!(result, Err(SagaSyncError::OutOfRange { .. })));
    }

    #[test]
    fn skips_unrecognized_topics() {
        let chunk = chunk_ref(0, 10, &[]);
        let body = format!("{}\n", fixture_line("0x1", "0x0"));

        let events = parse_chunk_events(body.as_bytes(), &chunk, 0..10).unwrap();
        assert!(events.is_empty());
    }
}
