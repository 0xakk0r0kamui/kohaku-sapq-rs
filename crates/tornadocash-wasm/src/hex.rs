use alloy_primitives::Address;
use kohaku_tornadocash::{Field, Nullifier, Secret};
use serde::{Deserialize, Serialize};
use tsify::Tsify;

#[derive(Serialize, Deserialize, Tsify)]
#[tsify(type = "`0x${string}`")]
pub struct Hex(String);

#[derive(Debug, thiserror::Error)]
pub enum HexError {
    #[error("Hex must start with 0x")]
    MissingPrefix,

    #[error("Invalid hex: {0}")]
    InvalidHex(#[from] hex::FromHexError),

    #[error("Invalid byte length: {0}")]
    InvalidLength(#[from] std::array::TryFromSliceError),
}

impl Hex {
    pub fn to_bytes(&self) -> Result<Vec<u8>, HexError> {
        let text = self.0.strip_prefix("0x").ok_or(HexError::MissingPrefix)?;

        Ok(hex::decode(text)?)
    }
}

impl TryFrom<Hex> for Address {
    type Error = HexError;

    fn try_from(value: Hex) -> Result<Self, Self::Error> {
        let bytes = value.to_bytes()?;
        let bytes: [u8; 20] = bytes.as_slice().try_into()?;
        Ok(Self::from(bytes))
    }
}

impl From<Address> for Hex {
    fn from(value: Address) -> Self {
        Self(value.to_string())
    }
}

impl TryFrom<Hex> for Nullifier {
    type Error = HexError;

    fn try_from(value: Hex) -> Result<Self, Self::Error> {
        let bytes = value.to_bytes()?;
        Ok(Nullifier::try_from(bytes.as_slice())?)
    }
}

impl TryFrom<Hex> for Secret {
    type Error = HexError;

    fn try_from(value: Hex) -> Result<Self, Self::Error> {
        let bytes = value.to_bytes()?;
        Ok(Secret::try_from(bytes.as_slice())?)
    }
}

impl From<Nullifier> for Hex {
    fn from(value: Nullifier) -> Self {
        Self(format!("0x{}", hex::encode(value.as_bytes())))
    }
}

impl From<Secret> for Hex {
    fn from(value: Secret) -> Self {
        Self(format!("0x{}", hex::encode(value.as_bytes())))
    }
}

impl From<&[u8]> for Hex {
    fn from(value: &[u8]) -> Self {
        Self(format!("0x{}", hex::encode(value)))
    }
}

impl From<Field> for Hex {
    fn from(value: Field) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;

    use super::{Hex, HexError};

    #[test]
    fn converts_address_and_hex_preserving_leading_zeros() {
        let address = Address::with_last_byte(0xab);
        let text = "0x00000000000000000000000000000000000000AB";

        assert_eq!(Hex::from(address).0, text);
        assert_eq!(Address::try_from(Hex(text.to_owned())).unwrap(), address);
    }

    #[test]
    fn decodes_address_without_requiring_checksum() {
        for text in [
            "0x00000000000000000000000000000000000000ab",
            "0x00000000000000000000000000000000000000aB",
        ] {
            assert_eq!(
                Address::try_from(Hex(text.to_owned())).unwrap(),
                Address::with_last_byte(0xab)
            );
        }
    }

    #[test]
    fn rejects_address_with_incorrect_byte_length() {
        for length in [0, 19, 21] {
            let value = Hex(format!("0x{}", "ab".repeat(length)));

            assert!(matches!(
                Address::try_from(value),
                Err(HexError::InvalidLength(_))
            ));
        }
    }

    #[test]
    fn converts_hex_and_bytes_preserving_leading_zeros() {
        let bytes = [0x00, 0x01, 0xff];
        let text = "0x0001ff";

        assert_eq!(Hex(text.to_owned()).to_bytes().unwrap(), bytes);
        assert_eq!(Hex::from(bytes.as_slice()).0, text);
    }

    #[test]
    fn rejects_missing_prefix() {
        let value = Hex("01".repeat(31));

        assert!(matches!(value.to_bytes(), Err(HexError::MissingPrefix)));
    }
}
