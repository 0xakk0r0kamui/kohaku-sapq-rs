use alloy::primitives::Address;
use rand::{CryptoRng, RngExt};
use zeroize::Zeroizing;

use crate::scheme;

pub use crate::scheme::SchemeError;

pub const SCHEME_ID: u64 = scheme::SCHEME_ID;

const RANDOM_ANNOUNCEMENT_ATTEMPTS: usize = 4;

pub struct AccountSeed(Zeroizing<[u8; 128]>);

impl AccountSeed {
    #[must_use]
    pub fn generate(rng: &mut impl CryptoRng) -> Self {
        Self(Zeroizing::new(rng.random()))
    }

    #[must_use]
    pub fn from_bytes(bytes: [u8; 128]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    #[must_use]
    pub fn expose_secret(&self) -> &[u8; 128] {
        &self.0
    }
}

impl core::fmt::Debug for AccountSeed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AccountSeed([REDACTED])")
    }
}

impl AsRef<[u8]> for AccountSeed {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StealthMetaAddress(Vec<u8>);

impl StealthMetaAddress {
    /// Parses and validates a scheme 3 ERC-6538 meta-address.
    ///
    /// # Errors
    ///
    /// Returns [`SchemeError::Malformed`] when `bytes` is not a valid scheme 3 encoding.
    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Result<Self, SchemeError> {
        let bytes = bytes.as_ref();
        if !scheme::meta_address_is_valid(bytes) {
            return Err(SchemeError::Malformed);
        }
        Ok(Self(bytes.to_vec()))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl core::fmt::Debug for StealthMetaAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StealthMetaAddress")
            .field("scheme_id", &SCHEME_ID)
            .field("length", &self.0.len())
            .finish()
    }
}

pub struct MasterKey(scheme::Master);

impl core::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MasterKey([REDACTED])")
    }
}

pub struct TrackingKey(scheme::Tracking);

impl core::fmt::Debug for TrackingKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TrackingKey([REDACTED])")
    }
}

pub struct Scanner(scheme::Scanner);

impl core::fmt::Debug for Scanner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Scanner([REDACTED])")
    }
}

pub struct Scheme3Account {
    meta_address: StealthMetaAddress,
    master_key: MasterKey,
    tracking_key: TrackingKey,
}

impl Scheme3Account {
    /// Derives an account from the scheme's 128-byte key-generation seed.
    ///
    /// # Errors
    ///
    /// Returns an engine error when the seed has the wrong length or contains invalid key
    /// material.
    pub fn from_seed(seed: &[u8]) -> Result<Self, SchemeError> {
        let keys = scheme::keygen(seed)?;
        Ok(Self {
            meta_address: StealthMetaAddress(keys.meta_address),
            master_key: MasterKey(keys.master),
            tracking_key: TrackingKey(keys.tracking),
        })
    }

    #[must_use]
    pub fn meta_address(&self) -> &StealthMetaAddress {
        &self.meta_address
    }

    #[must_use]
    pub fn master_key(&self) -> &MasterKey {
        &self.master_key
    }

    #[must_use]
    pub fn tracking_key(&self) -> &TrackingKey {
        &self.tracking_key
    }

    /// Binds the tracking key to this account's registered meta-address.
    ///
    /// # Errors
    ///
    /// Returns an error if the account key material is inconsistent.
    pub fn scanner(&self) -> Result<Scanner, SchemeError> {
        bind_scanner(&self.tracking_key, &self.meta_address)
    }

    /// Derives the one-time private key for a matched announcement.
    ///
    /// # Errors
    ///
    /// Returns an error if this account does not control the matched stealth address.
    pub fn derive_stealth_private_key(
        &self,
        matched: &MatchedAnnouncement,
    ) -> Result<StealthPrivateKey, SchemeError> {
        derive_stealth_private_key(&self.master_key, matched)
    }
}

impl core::fmt::Debug for Scheme3Account {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Scheme3Account")
            .field("meta_address", &self.meta_address)
            .field("master_key", &self.master_key)
            .field("tracking_key", &self.tracking_key)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Announcement {
    stealth_address: Address,
    ephemeral_public_key: Vec<u8>,
    metadata: Vec<u8>,
}

impl Announcement {
    /// Parses and validates scheme 3 ERC-5564 announcement fields.
    ///
    /// # Errors
    ///
    /// Returns [`SchemeError::Malformed`] when a field is not a valid scheme 3 encoding.
    pub fn from_parts(
        stealth_address: Address,
        ephemeral_public_key: impl Into<Vec<u8>>,
        metadata: impl Into<Vec<u8>>,
    ) -> Result<Self, SchemeError> {
        let ephemeral_public_key = ephemeral_public_key.into();
        let metadata = metadata.into();
        if !scheme::announcement_is_valid(
            &stealth_address.into_array(),
            &ephemeral_public_key,
            &metadata,
        ) {
            return Err(SchemeError::Malformed);
        }
        Ok(Self {
            stealth_address,
            ephemeral_public_key,
            metadata,
        })
    }

    #[must_use]
    pub const fn stealth_address(&self) -> Address {
        self.stealth_address
    }

    #[must_use]
    pub fn ephemeral_public_key(&self) -> &[u8] {
        &self.ephemeral_public_key
    }

    #[must_use]
    pub fn metadata(&self) -> &[u8] {
        &self.metadata
    }

    fn from_engine(value: scheme::Announcement) -> Self {
        debug_assert_eq!(value.scheme_id, SCHEME_ID);
        Self {
            stealth_address: Address::from(value.stealth_address),
            ephemeral_public_key: value.ephemeral_pub_key,
            metadata: value.metadata,
        }
    }
}

impl core::fmt::Debug for Announcement {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Announcement")
            .field("scheme_id", &SCHEME_ID)
            .field("stealth_address", &self.stealth_address)
            .field("ephemeral_public_key_len", &self.ephemeral_public_key.len())
            .field("metadata_len", &self.metadata.len())
            .finish()
    }
}

#[derive(Debug)]
pub struct GeneratedStealthAddress {
    pub stealth_address: Address,
    pub announcement: Announcement,
}

pub struct MatchedAnnouncement(scheme::Match);

impl MatchedAnnouncement {
    #[must_use]
    pub fn stealth_address(&self) -> Address {
        Address::from(self.0.stealth_address)
    }
}

impl core::fmt::Debug for MatchedAnnouncement {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MatchedAnnouncement")
            .field("stealth_address", &self.stealth_address())
            .finish_non_exhaustive()
    }
}

pub struct StealthPrivateKey(Zeroizing<[u8; 32]>);

impl StealthPrivateKey {
    #[must_use]
    pub fn expose_secret(&self) -> &[u8; 32] {
        &self.0
    }
}

impl core::fmt::Debug for StealthPrivateKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StealthPrivateKey([REDACTED])")
    }
}

/// Generates fresh announcement material and a stealth address.
///
/// # Errors
///
/// Returns an engine error if the meta-address is invalid or fresh valid announcement material
/// cannot be generated.
pub fn generate_stealth_address(
    meta_address: &StealthMetaAddress,
    rng: &mut impl CryptoRng,
) -> Result<GeneratedStealthAddress, SchemeError> {
    let mut last_error = SchemeError::SeedRejected;
    for _ in 0..RANDOM_ANNOUNCEMENT_ATTEMPTS {
        let seed = Zeroizing::new(rng.random::<[u8; 64]>());
        match generate_stealth_address_with_seed(meta_address, seed.as_slice()) {
            Ok(generated) => return Ok(generated),
            Err(error @ SchemeError::SeedRejected) => last_error = error,
            Err(error) => return Err(error),
        }
    }
    Err(last_error)
}

/// Generates deterministic announcement material from the scheme's 64-byte announcement seed.
/// Reusing a seed repeats the stealth address.
///
/// # Errors
///
/// Returns an engine error when the seed or meta-address is invalid.
pub fn generate_stealth_address_with_seed(
    meta_address: &StealthMetaAddress,
    seed: &[u8],
) -> Result<GeneratedStealthAddress, SchemeError> {
    let announcement =
        Announcement::from_engine(scheme::announce_with_seed(meta_address.as_bytes(), seed)?);
    Ok(GeneratedStealthAddress {
        stealth_address: announcement.stealth_address(),
        announcement,
    })
}

/// Binds delegated tracking material to a registered meta-address.
///
/// # Errors
///
/// Returns an error if the tracking key does not belong to the meta-address.
pub fn bind_scanner(
    tracking_key: &TrackingKey,
    meta_address: &StealthMetaAddress,
) -> Result<Scanner, SchemeError> {
    scheme::bind(&tracking_key.0, meta_address.as_bytes()).map(Scanner)
}

#[must_use]
pub fn try_match_announcement(
    scanner: &Scanner,
    announcement: &Announcement,
) -> Option<MatchedAnnouncement> {
    scheme::check(
        &scanner.0,
        SCHEME_ID,
        &announcement.stealth_address.into_array(),
        &announcement.ephemeral_public_key,
        &announcement.metadata,
    )
    .map(MatchedAnnouncement)
}

/// Derives the one-time private key for a matched announcement.
///
/// # Errors
///
/// Returns an error if `master_key` does not control the matched stealth address.
pub fn derive_stealth_private_key(
    master_key: &MasterKey,
    matched: &MatchedAnnouncement,
) -> Result<StealthPrivateKey, SchemeError> {
    scheme::spend_key(&master_key.0, &matched.0).map(StealthPrivateKey)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_secret_types_redact_debug_output() {
        let account_seed = AccountSeed::from_bytes([0x7f; 128]);
        let private_key = StealthPrivateKey(Zeroizing::new([0x6a; 32]));

        assert_eq!(format!("{account_seed:?}"), "AccountSeed([REDACTED])");
        assert_eq!(format!("{private_key:?}"), "StealthPrivateKey([REDACTED])");
    }
}
