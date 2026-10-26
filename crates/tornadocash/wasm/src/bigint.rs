use alloy::primitives::U256;
use js_sys::BigInt;
use wasm_bindgen::prelude::*;

// These types describe JS primitives, not exported wrapper classes. Conversion
// into a Rust integer validates the original value before any numeric coercion.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "number")]
    pub type JsU8;
    #[wasm_bindgen(typescript_type = "number")]
    pub type JsU32;
    #[wasm_bindgen(typescript_type = "bigint")]
    pub type JsU64;
    #[wasm_bindgen(typescript_type = "bigint")]
    pub type JsU128;
    #[wasm_bindgen(typescript_type = "bigint")]
    pub type JsU256;
}

impl TryFrom<JsU8> for u8 {
    type Error = JsError;

    fn try_from(value: JsU8) -> Result<Self, Self::Error> {
        let error = || JsError::new("value must be a finite integer number in the u8 range");
        let value = value.as_f64().ok_or_else(error)?;
        if !value.is_finite()
            || value.fract() != 0.0
            || !(0.0..=f64::from(Self::MAX)).contains(&value)
        {
            return Err(error());
        }
        #[expect(
            clippy::cast_sign_loss,
            reason = "validated as an integer in the u8 range"
        )]
        Ok(value as Self)
    }
}

impl From<u8> for JsU8 {
    fn from(value: u8) -> Self {
        JsValue::from(value).unchecked_into()
    }
}

impl TryFrom<JsU32> for u32 {
    type Error = JsError;

    fn try_from(value: JsU32) -> Result<Self, Self::Error> {
        let error = || JsError::new("value must be a finite integer number in the u32 range");
        let value = value.as_f64().ok_or_else(error)?;
        if !value.is_finite()
            || value.fract() != 0.0
            || !(0.0..=f64::from(Self::MAX)).contains(&value)
        {
            return Err(error());
        }
        #[expect(
            clippy::cast_sign_loss,
            reason = "validated as an integer in the u32 range"
        )]
        Ok(value as Self)
    }
}

impl From<u32> for JsU32 {
    fn from(value: u32) -> Self {
        JsValue::from(value).unchecked_into()
    }
}

impl TryFrom<JsU64> for u64 {
    type Error = JsError;

    fn try_from(value: JsU64) -> Result<Self, Self::Error> {
        // Unlike the generated primitive argument conversion, TryFrom<JsValue>
        // checks the bigint range rather than wrapping it.
        Self::try_from(JsValue::from(value))
            .map_err(|_| JsError::new("value must be a bigint in the u64 range"))
    }
}

impl From<u64> for JsU64 {
    fn from(value: u64) -> Self {
        JsValue::from(value).unchecked_into()
    }
}

impl TryFrom<JsU128> for u128 {
    type Error = JsError;

    fn try_from(value: JsU128) -> Result<Self, Self::Error> {
        Self::try_from(JsValue::from(value))
            .map_err(|_| JsError::new("value must be a bigint in the u128 range"))
    }
}

impl From<u128> for JsU128 {
    fn from(value: u128) -> Self {
        JsValue::from(value).unchecked_into()
    }
}

impl TryFrom<JsU256> for U256 {
    type Error = JsError;

    fn try_from(value: JsU256) -> Result<Self, Self::Error> {
        let error = || JsError::new("value must be a bigint in the U256 range");
        if !value.is_bigint() {
            return Err(error());
        }
        let decimal = value
            .unchecked_ref::<BigInt>()
            .to_string(10)
            .map_err(|_| error())?;
        Self::from_str_radix(&String::from(decimal), 10).map_err(|_| error())
    }
}

impl From<U256> for JsU256 {
    fn from(value: U256) -> Self {
        JsValue::bigint_from_str(&value.to_string()).unchecked_into()
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test]
    fn number_conversions_check_their_widths() {
        for value in [0, u8::MAX] {
            assert_eq!(u8::try_from(JsU8::from(value)).unwrap(), value);
        }
        for value in [0, u32::MAX] {
            assert_eq!(u32::try_from(JsU32::from(value)).unwrap(), value);
        }
        assert!(u8::try_from(JsValue::from_f64(256.0).unchecked_into::<JsU8>()).is_err());
        assert!(
            u32::try_from(JsValue::from_f64(f64::from(u32::MAX) + 1.0).unchecked_into::<JsU32>())
                .is_err()
        );
    }

    #[wasm_bindgen_test]
    fn number_conversions_reject_coercion_and_truncation() {
        for value in [
            JsValue::from_f64(-1.0),
            JsValue::from_f64(1.9),
            JsValue::from_f64(f64::NAN),
            JsValue::from_f64(f64::INFINITY),
            JsValue::from_str("1"),
            JsValue::from(1_u64),
            JsValue::NULL,
            JsValue::UNDEFINED,
        ] {
            assert!(u8::try_from(value.clone().unchecked_into::<JsU8>()).is_err());
            assert!(u32::try_from(value.unchecked_into::<JsU32>()).is_err());
        }
    }

    #[wasm_bindgen_test]
    fn bigint_widths_use_checked_conversions() {
        assert_eq!(u64::try_from(JsU64::from(u64::MAX)).unwrap(), u64::MAX);
        assert_eq!(u128::try_from(JsU128::from(u128::MAX)).unwrap(), u128::MAX);
        assert!(
            u64::try_from(
                JsValue::bigint_from_str("18446744073709551616").unchecked_into::<JsU64>()
            )
            .is_err()
        );
        assert!(
            u128::try_from(
                JsValue::bigint_from_str("340282366920938463463374607431768211456")
                    .unchecked_into::<JsU128>()
            )
            .is_err()
        );
        for value in [JsValue::from(-1_i64), JsValue::from_f64(1.0)] {
            assert!(u64::try_from(value.clone().unchecked_into::<JsU64>()).is_err());
            assert!(u128::try_from(value.unchecked_into::<JsU128>()).is_err());
        }
    }

    #[wasm_bindgen_test]
    fn converts_exact_unsigned_256_bit_bigints() {
        for value in [U256::ZERO, U256::from(1) << 200, U256::MAX] {
            let output = JsU256::from(value);
            assert_eq!(
                output.unchecked_ref::<BigInt>().to_string(10).unwrap(),
                value.to_string()
            );
            assert_eq!(U256::try_from(output).unwrap(), value);
        }
    }

    #[wasm_bindgen_test]
    fn rejects_out_of_range_u256_bigints() {
        for input in [
            "-1",
            "115792089237316195423570985008687907853269984665640564039457584007913129639936",
        ] {
            assert!(
                U256::try_from(JsValue::bigint_from_str(input).unchecked_into::<JsU256>()).is_err()
            );
        }
    }

    #[wasm_bindgen_test]
    fn rejects_non_bigint_u256_values_without_coercion() {
        for input in [
            JsValue::from_f64(1.0),
            JsValue::from_str("1"),
            JsValue::NULL,
            JsValue::UNDEFINED,
            JsValue::TRUE,
        ] {
            assert!(U256::try_from(input.unchecked_into::<JsU256>()).is_err());
        }
    }
}
