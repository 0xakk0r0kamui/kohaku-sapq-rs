use kohaku_tornadocash::Call as CoreCall;
use serde::Serialize;
use tsify::Tsify;

use crate::{bigint::JsU256, hex::Hex};

/// A contract call represented as plain JavaScript data.
///
/// See [`kohaku_tornadocash::Call`] for more details.
#[derive(Serialize, Tsify)]
pub struct Call {
    to: Hex,
    data: Hex,
    #[serde(with = "tsify::serde_wasm_bindgen::preserve")]
    #[tsify(type = "bigint")]
    value: JsU256,
}

impl From<CoreCall> for Call {
    fn from(call: CoreCall) -> Self {
        Self {
            to: call.target.into(),
            data: Hex::from(call.input.as_ref()),
            value: call.value.into(),
        }
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use alloy::primitives::{Address, Bytes, U256};
    use js_sys::Reflect;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test]
    fn maps_core_call_to_plain_js_fields() {
        let core = CoreCall {
            target: Address::with_last_byte(0xab),
            input: Bytes::from_static(&[0x00, 0xab]),
            value: U256::from(1) << 200,
        };
        let call: JsValue = Call::from(core.clone()).into_ts().unwrap().into();
        let to: JsValue = Hex::from(core.target).into_ts().unwrap().into();

        assert_eq!(Reflect::get(&call, &"to".into()).unwrap(), to);
        assert_eq!(
            Reflect::get(&call, &"data".into()).unwrap(),
            JsValue::from_str("0x00ab")
        );
        let value = Reflect::get(&call, &"value".into()).unwrap();
        assert_eq!(
            U256::try_from(value.unchecked_into::<JsU256>()).unwrap(),
            core.value
        );
    }
}
