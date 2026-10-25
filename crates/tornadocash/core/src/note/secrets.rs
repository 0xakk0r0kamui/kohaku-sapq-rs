use rand::RngExt;
use ruint::aliases::U256;
use serde::{Deserialize, Serialize};

macro_rules! bytes31_newtype {
    ($name:ident) => {
        #[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $name([u8; 31]);

        impl $name {
            pub const fn new(bytes: [u8; 31]) -> Self {
                Self(bytes)
            }
            pub const fn as_bytes(&self) -> &[u8; 31] {
                &self.0
            }
        }

        impl From<[u8; 31]> for $name {
            fn from(b: [u8; 31]) -> Self {
                Self(b)
            }
        }
        impl From<$name> for [u8; 31] {
            fn from(v: $name) -> Self {
                v.0
            }
        }
        impl From<$name> for U256 {
            fn from(v: $name) -> Self {
                U256::from_le_slice(&v.0)
            }
        }
        impl TryFrom<&[u8]> for $name {
            type Error = core::array::TryFromSliceError;
            fn try_from(s: &[u8]) -> Result<Self, Self::Error> {
                <[u8; 31]>::try_from(s).map(Self)
            }
        }

        impl rand::distr::Distribution<$name> for rand::distr::StandardUniform {
            fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> $name {
                $name(rng.random())
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}([REDACTED])", stringify!($name))
            }
        }
    };
}

bytes31_newtype!(Nullifier);
bytes31_newtype!(Secret);
