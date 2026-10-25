use std::{fmt::Display, str::FromStr};

use rand::{
    RngExt,
    distr::{Distribution, StandardUniform},
};
use serde::{Deserialize, Serialize};

pub use crate::note::secrets::{Nullifier, Secret};
use crate::{crypto::pedersen::pedersen_hash, field::Field, pool::Pool};

mod secrets;

/// Tornadocash deposit note.
///
/// Notes are produced when a user deposits funds into a tornadocash pool. They
/// contain the secret material required to later withdraw the funds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub nullifier: Nullifier,
    pub secret: Secret,
}

/// Displayable Tornadocash note.
///
/// Includes hints for the note's asset & pool. Parses from and formats the
/// standard Tornado Cash format.
///
/// ```text
/// tornado-{symbol}-{amount}-{chain_id}-0x{nullifier}{secret}
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteString {
    pub note: Note,
    pub symbol: String,
    pub amount: String,
    pub chain_id: u64,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NoteError {
    #[error("invalid note format")]
    InvalidFormat,
    #[error("invalid chain id")]
    InvalidChainId,
    #[error("invalid hex: {0}")]
    InvalidHex(#[from] hex::FromHexError),
}

impl Note {
    #[must_use]
    pub fn new(nullifier: impl Into<Nullifier>, secret: impl Into<Secret>) -> Note {
        Note {
            nullifier: nullifier.into(),
            secret: secret.into(),
        }
    }

    #[must_use]
    pub fn preimage(&self) -> [u8; 62] {
        let mut buf = [0u8; 62];
        buf[..31].copy_from_slice(self.nullifier.as_bytes());
        buf[31..].copy_from_slice(self.secret.as_bytes());
        buf
    }

    #[must_use]
    pub fn commitment(&self) -> Field {
        Field::from_fr(pedersen_hash(&self.preimage()))
    }

    #[must_use]
    pub fn nullifier_hash(&self) -> Field {
        Field::from_fr(pedersen_hash(self.nullifier.as_bytes()))
    }
}

impl NoteString {
    #[must_use]
    pub fn new(
        note: Note,
        symbol: impl Into<String>,
        amount: impl Into<String>,
        chain_id: u64,
    ) -> Self {
        Self {
            note,
            symbol: symbol.into(),
            amount: amount.into(),
            chain_id,
        }
    }

    #[must_use]
    pub fn from_pool(note: Note, pool: &Pool) -> Self {
        Self {
            note,
            symbol: pool.symbol().to_string(),
            amount: pool.amount().clone(),
            chain_id: pool.chain_id,
        }
    }

    #[must_use]
    pub fn preimage(&self) -> [u8; 62] {
        self.note.preimage()
    }

    #[must_use]
    pub fn commitment(&self) -> Field {
        self.note.commitment()
    }

    #[must_use]
    pub fn nullifier_hash(&self) -> Field {
        self.note.nullifier_hash()
    }
}

impl Distribution<Note> for StandardUniform {
    fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> Note {
        Note {
            nullifier: rng.random(),
            secret: rng.random(),
        }
    }
}

impl Display for NoteString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tornado-{}-{}-{}-0x{}",
            self.symbol,
            self.amount,
            self.chain_id,
            hex::encode(self.note.preimage())
        )
    }
}

impl FromStr for NoteString {
    type Err = NoteError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Format: tornado-{symbol}-{amount}-{chain_id}-0x{124-char hex}
        let parts: Vec<&str> = s.splitn(5, '-').collect();
        if parts.len() != 5 || parts[0] != "tornado" {
            return Err(NoteError::InvalidFormat);
        }

        let symbol = parts[1].to_string();
        let amount = parts[2].to_string();
        let chain_id: u64 = parts[3].parse().map_err(|_| NoteError::InvalidChainId)?;

        let hex_str = parts[4].strip_prefix("0x").unwrap_or(parts[4]);
        let bytes = hex::decode(hex_str)?;
        if bytes.len() != 62 {
            return Err(NoteError::InvalidFormat);
        }

        let mut nullifier = [0u8; 31];
        let mut secret = [0u8; 31];
        nullifier.copy_from_slice(&bytes[..31]);
        secret.copy_from_slice(&bytes[31..]);

        Ok(NoteString::new(
            Note::new(nullifier, secret),
            symbol,
            amount,
            chain_id,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_encoding_decoding() {
        let nullifier = [1u8; 31];
        let secret = [2u8; 31];
        let symbol = "eth";
        let amount = "1";
        let chain_id = 1;
        let note = NoteString::new(Note::new(nullifier, secret), symbol, amount, chain_id);
        let encoded = note.to_string();

        let expected = "tornado-eth-1-1-0x0101010101010101010101010101010101010101010101010101010101010102020202020202020202020202020202020202020202020202020202020202";
        assert_eq!(encoded, expected);

        let decoded_note = NoteString::from_str(&encoded).unwrap();
        assert_eq!(note, decoded_note);
    }
}
