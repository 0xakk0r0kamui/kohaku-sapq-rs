use std::collections::HashMap;

use alloy::primitives::{Address, B256, U64};
use serde::{Deserialize, Deserializer};

/// The saga-sync manifest (`index.json`).
///
/// A map of stream key to its published chunks. See the documentation for more details:
/// <https://github.com/fatlabsxyz/saga-sync/blob/master/SPEC.md#31-manifest>
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub available_protocols: HashMap<String, StreamEntry>,
}

#[expect(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamEntry {
    pub protocol: String,
    #[serde(deserialize_with = "deserialize_hex_u64")]
    pub chain_id: u64,
    pub tracked_addresses: Vec<Address>,
    pub tracked_event_topics: Vec<B256>,
    pub chunks: Vec<ChunkRef>,
    pub hot_head: Option<ChunkRef>,
}

#[expect(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkRef {
    #[serde(deserialize_with = "deserialize_hex_u64")]
    pub from_block: u64,
    #[serde(deserialize_with = "deserialize_hex_u64")]
    pub to_block: u64,
    pub file: String,
    #[serde(deserialize_with = "deserialize_hex_u64")]
    pub size: u64,
    pub digest: Digest,
}

#[expect(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Digest {
    #[serde(rename = "type")]
    pub kind: String,
    pub data: String,
}

impl StreamEntry {
    /// The last block covered by this stream.
    #[must_use]
    pub fn last_block(&self) -> Option<u64> {
        self.hot_head
            .as_ref()
            .map(|c| c.to_block)
            .or_else(|| self.chunks.last().map(|c| c.to_block))
    }
}

/// Deserializes a `0x`-prefixed hex-encoded quantity (e.g. `"0x64"`) into a `u64`.
pub fn deserialize_hex_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(U64::deserialize(d)?.to::<u64>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_hex_fields() {
        let json = r#"{
            "fromBlock": "0x64",
            "toBlock": "0xc8",
            "file": "chunk.jsonl.gz",
            "size": "0x1a2b",
            "digest": { "type": "sha256", "data": "0xabcdef" }
        }"#;
        let chunk: ChunkRef = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.from_block, 100);
        assert_eq!(chunk.to_block, 200);
    }

    #[test]
    fn resolves_last_block_from_hot_head_then_sealed_chunk() {
        let sealed = ChunkRef {
            from_block: 0,
            to_block: 100,
            file: "a".into(),
            size: 0,
            digest: Digest {
                kind: "sha256".into(),
                data: "0x00".into(),
            },
        };

        let with_hot_head = StreamEntry {
            protocol: "tornado-cash".into(),
            chain_id: 1,
            tracked_addresses: vec![],
            tracked_event_topics: vec![],
            chunks: vec![sealed.clone()],
            hot_head: Some(ChunkRef {
                from_block: 100,
                to_block: 150,
                ..sealed.clone()
            }),
        };
        assert_eq!(with_hot_head.last_block(), Some(150));

        let sealed_only = StreamEntry {
            hot_head: None,
            ..with_hot_head
        };
        assert_eq!(sealed_only.last_block(), Some(100));

        let empty = StreamEntry {
            chunks: vec![],
            ..sealed_only
        };
        assert_eq!(empty.last_block(), None);
    }
}
