use std::fmt::{Debug, Display};

use alloy::primitives::B256;
use ark_bn254::Fr;
use ruint::aliases::U256;
use serde::{Deserialize, Serialize};

/// Tornadocash BN254 field element.
#[derive(Copy, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Field(Fr);

#[derive(Debug, thiserror::Error)]
#[error("Field not in range")]
pub struct FieldNotInRangeError;

impl Field {
    pub(crate) fn from_fr(fr: Fr) -> Self {
        Self(fr)
    }

    pub(crate) fn as_fr(&self) -> Fr {
        self.0
    }
}

impl From<Field> for U256 {
    fn from(value: Field) -> Self {
        value.0.into()
    }
}

impl From<Field> for B256 {
    fn from(value: Field) -> Self {
        let u256: U256 = value.into();
        u256.into()
    }
}

impl From<u32> for Field {
    fn from(value: u32) -> Self {
        let fr = Fr::from(value);
        Self(fr)
    }
}

impl TryFrom<U256> for Field {
    type Error = FieldNotInRangeError;

    fn try_from(value: U256) -> Result<Self, Self::Error> {
        match value.try_into() {
            Ok(fr) => Ok(Field(fr)),
            Err(_) => Err(FieldNotInRangeError),
        }
    }
}

impl TryFrom<B256> for Field {
    type Error = FieldNotInRangeError;

    fn try_from(value: B256) -> Result<Self, Self::Error> {
        let u256: U256 = value.into();
        u256.try_into()
    }
}

impl Debug for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let u256: U256 = (*self).into();
        write!(f, "0x{}", hex::encode(u256.to_be_bytes::<32>()))
    }
}

impl Serialize for Field {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let b256: B256 = (*self).into();
        b256.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Field {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let b256 = B256::deserialize(deserializer)?;
        b256.try_into().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_from_u256() {
        let u256 = U256::from(123_456_789u64);
        let field = Field::try_from(u256).unwrap();
        let back_to_u256: U256 = field.into();
        assert_eq!(u256, back_to_u256);
    }

    #[test]
    fn field_from_b256() {
        let b256: B256 = U256::from(123_456_789u64).into();
        let field = Field::try_from(b256).unwrap();
        let back_to_b256: B256 = field.into();
        assert_eq!(b256, back_to_b256);
    }

    #[test]
    fn field_from_u256_overflows() {
        let u256 = U256::MAX;
        assert!(Field::try_from(u256).is_err());
    }

    #[test]
    fn field_from_b256_overflows() {
        let b256 = B256::from([255u8; 32]);
        assert!(Field::try_from(b256).is_err());
    }

    #[test]
    fn serialize_deserialize_field() {
        let field = Field::from(123_456_789u32);
        let serialized = serde_json::to_string(&field).unwrap();
        let deserialized: Field = serde_json::from_str(&serialized).unwrap();
        assert_eq!(field, deserialized);
    }

    #[test]
    fn display_field() {
        let field = Field::from(123_456_789u32);
        let display = format!("{field}");
        assert_eq!(
            display,
            "0x00000000000000000000000000000000000000000000000000000000075bcd15"
        );
    }
}
