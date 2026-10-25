use alloy::{
    primitives::{Address, B256},
    rpc::types::Log,
};
use kohaku_tornadocash::event::SyncEvent;
use serde::{Deserialize, Serialize};
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::{
    bigint::{JsU32, JsU64, JsU256},
    hex::Hex,
};

/// Caller-supplied log data, without emitter or reorg metadata.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct LogInput {
    topics: Vec<Hex>,
    data: Hex,
    // Preserve the JS type so the shared helper can reject numbers and null.
    #[serde(default, with = "tsify::serde_wasm_bindgen::preserve")]
    #[tsify(optional, type = "bigint")]
    block_number: JsValue,
}

/// A plain JavaScript snapshot of [`SyncEvent`].
#[derive(Serialize, Tsify)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DecodedEvent {
    Deposit {
        commitment: Hex,
        #[serde(with = "tsify::serde_wasm_bindgen::preserve")]
        #[tsify(type = "number")]
        leaf_index: JsU32,
        #[serde(with = "tsify::serde_wasm_bindgen::preserve")]
        #[tsify(type = "bigint")]
        block_number: JsU64,
    },
    Withdrawal {
        to: Hex,
        nullifier_hash: Hex,
        relayer: Hex,
        #[serde(with = "tsify::serde_wasm_bindgen::preserve")]
        #[tsify(type = "bigint")]
        fee: JsU256,
        #[serde(with = "tsify::serde_wasm_bindgen::preserve")]
        #[tsify(type = "bigint")]
        block_number: JsU64,
    },
}

/// Decode a supplied log using [`SyncEvent::try_from_log`].
///
/// Returns the decoded event or undefined for any core no-result, including an
/// unknown/undecodable event or absent block number.
///
/// # Errors
///
/// Throws a JavaScript Error for malformed input: topics must be 32-byte Hex
/// strings with at most four entries, data must be Hex bytes, and a present
/// blockNumber must be an unsigned u64 bigint.
#[wasm_bindgen(js_name = decodeEvent)]
pub fn decode_event(log: &Ts<LogInput>) -> Result<Option<Ts<DecodedEvent>>, JsError> {
    let log = log.to_rust()?.into_log()?;
    match SyncEvent::try_from_log(&log) {
        Some(event) => Ok(Some(DecodedEvent::from(event).into_ts()?)),
        None => Ok(None),
    }
}

impl From<SyncEvent> for DecodedEvent {
    fn from(event: SyncEvent) -> Self {
        match event {
            SyncEvent::Deposit(event) => Self::Deposit {
                commitment: event.commitment.into(),
                leaf_index: event.leaf_index.into(),
                block_number: event.block_number.into(),
            },
            SyncEvent::Withdrawal(event) => Self::Withdrawal {
                to: event.to.into(),
                nullifier_hash: event.nullifier_hash.into(),
                relayer: event.relayer.into(),
                fee: event.fee.into(),
                block_number: event.block_number.into(),
            },
        }
    }
}

impl LogInput {
    fn into_log(self) -> Result<Log, JsError> {
        let topics = self
            .topics
            .into_iter()
            .map(|topic| {
                let bytes = topic.to_bytes()?;
                let bytes: [u8; 32] = bytes.as_slice().try_into()?;
                Ok(B256::from(bytes))
            })
            .collect::<Result<Vec<_>, JsError>>()?;
        let data = self.data.to_bytes()?.into();
        // The input does not identify an emitter. Core decoding uses only topics/data.
        let inner = alloy::primitives::Log::new(Address::ZERO, topics, data)
            .ok_or_else(|| JsError::new("Log topics must contain at most 4 entries"))?;

        let block_number = (!self.block_number.is_undefined())
            .then(|| self.block_number.unchecked_into::<JsU64>())
            .map(u64::try_from)
            .transpose()?;

        Ok(Log {
            inner,
            block_number,
            ..Log::default()
        })
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use alloy::primitives::{Address, U256};
    use js_sys::{Array, Object, Reflect};
    use kohaku_tornadocash::{
        Field,
        event::{DepositEvent, WithdrawalEvent},
    };
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test]
    fn maps_deposit_fields() {
        let expected_commitment = Field::from(0xab);
        let event = SyncEvent::Deposit(DepositEvent {
            commitment: expected_commitment,
            leaf_index: 7,
            block_number: 9_007_199_254_740_993,
        });
        let DecodedEvent::Deposit {
            commitment,
            leaf_index,
            block_number,
        } = DecodedEvent::from(event)
        else {
            panic!("expected deposit snapshot");
        };

        assert_eq!(Field::try_from(commitment).unwrap(), expected_commitment);
        assert_eq!(u32::try_from(leaf_index).unwrap(), 7);
        assert_eq!(u64::try_from(block_number).unwrap(), 9_007_199_254_740_993);
    }

    #[wasm_bindgen_test]
    fn maps_withdrawal_fields() {
        let recipient = Address::with_last_byte(0xab);
        let expected_relayer = Address::with_last_byte(0xcd);
        let expected_nullifier = Field::from(7);
        let expected_fee = (U256::from(1) << 200) + U256::from(17);
        let event = SyncEvent::Withdrawal(WithdrawalEvent {
            to: recipient,
            nullifier_hash: expected_nullifier,
            relayer: expected_relayer,
            fee: expected_fee,
            block_number: 42,
        });
        let DecodedEvent::Withdrawal {
            to,
            nullifier_hash,
            relayer,
            fee,
            block_number,
        } = DecodedEvent::from(event)
        else {
            panic!("expected withdrawal snapshot");
        };

        assert_eq!(Address::try_from(to).unwrap(), recipient);
        assert_eq!(Field::try_from(nullifier_hash).unwrap(), expected_nullifier);
        assert_eq!(Address::try_from(relayer).unwrap(), expected_relayer);
        assert_eq!(U256::try_from(fee).unwrap(), expected_fee);
        assert_eq!(u64::try_from(block_number).unwrap(), 42);
    }

    #[wasm_bindgen_test]
    fn adapts_log_bytes_and_checks_topic_width() {
        let log = LogInput {
            topics: vec![Hex::from([0xab; 32].as_slice())],
            data: Hex::from([0x00, 0x01, 0xff].as_slice()),
            block_number: JsValue::from(7_u64),
        }
        .into_log()
        .unwrap();

        assert_eq!(log.topics(), &[B256::repeat_byte(0xab)]);
        assert_eq!(log.data().data.as_ref(), &[0x00, 0x01, 0xff]);
        assert_eq!(log.block_number, Some(7));
        assert!(
            LogInput {
                topics: vec![Hex::from([0xab; 31].as_slice())],
                data: Hex::from([].as_slice()),
                block_number: JsValue::UNDEFINED,
            }
            .into_log()
            .is_err()
        );
    }

    #[wasm_bindgen_test]
    fn preserves_consumer_block_optionality_and_core_none() {
        let value = Object::new();
        Reflect::set(&value, &"topics".into(), &Array::new()).unwrap();
        Reflect::set(&value, &"data".into(), &"0x".into()).unwrap();
        let input = Ts::new_unchecked(value.clone().into());

        assert!(decode_event(&input).unwrap().is_none());
        Reflect::set(&value, &"blockNumber".into(), &JsValue::NULL).unwrap();
        assert!(decode_event(&input).is_err());
        Reflect::set(&value, &"blockNumber".into(), &JsValue::from(7_u64)).unwrap();
        assert!(decode_event(&input).unwrap().is_none());
    }
}
