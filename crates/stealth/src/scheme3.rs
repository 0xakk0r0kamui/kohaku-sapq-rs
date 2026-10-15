use alloy::primitives::Address;
use pqsa_core::{StealthScheme, keygen_seed};
use pqsa_per_payment::SchemeId3;
use rand::CryptoRng;
use thiserror::Error;
use tracing::{debug, trace};
use zeroize::Zeroizing;

pub use pqsa_core::Error as EngineError;
pub use pqsa_per_payment::{
    Announcement as StealthAddress, Master, Match, MetaAddress as StealthMetaAddress, Scanner,
    SpendKey, Tracking,
};
pub use secrecy::ExposeSecret;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SchemeError {
    #[error("scheme 3 engine error: {0:?}")]
    Engine(EngineError),
    #[error("key generation retry limit reached")]
    KeygenExhausted,
}

impl From<EngineError> for SchemeError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

pub const SCHEME_ID: u64 = SchemeId3::SCHEME_ID;

const KEYGEN_ATTEMPTS: u64 = 1_024;
const RANDOM_ANNOUNCEMENT_ATTEMPTS: usize = 4;

pub struct AccountSeed(Zeroizing<[u8; 128]>);

impl AccountSeed {
    #[must_use]
    pub fn generate(rng: &mut impl CryptoRng) -> Self {
        let mut seed = Self(Zeroizing::new([0u8; 128]));
        rng.fill_bytes(seed.0.as_mut());
        seed
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

#[derive(Debug)]
pub struct Scheme3Account {
    meta_address: StealthMetaAddress,
    master_key: Master,
    tracking_key: Tracking,
}

#[derive(Debug)]
pub struct DerivedScheme3Account {
    pub account: Scheme3Account,
    pub keygen_index: u64,
}

impl Scheme3Account {
    /// Derives the first valid account from a 32-byte master key.
    ///
    /// # Errors
    ///
    /// Returns an engine error for invalid input, or [`SchemeError::KeygenExhausted`]
    /// after `KEYGEN_ATTEMPTS` attempts.
    pub fn from_keygen_master(master: &[u8]) -> Result<DerivedScheme3Account, SchemeError> {
        for keygen_index in 0..KEYGEN_ATTEMPTS {
            match Self::from_keygen_master_at(master, keygen_index) {
                Ok(account) => {
                    return Ok(DerivedScheme3Account {
                        account,
                        keygen_index,
                    });
                }
                Err(SchemeError::Engine(
                    EngineError::NoValidScalar | EngineError::SpendingKeyDelegated,
                )) => {}
                Err(error) => return Err(error),
            }
        }
        Err(SchemeError::KeygenExhausted)
    }

    /// Restores the account derived at `keygen_index`.
    pub fn from_keygen_master_at(master: &[u8], keygen_index: u64) -> Result<Self, SchemeError> {
        let seed = keygen_seed(
            master,
            SCHEME_ID,
            SchemeId3::NAME.as_bytes(),
            keygen_index,
            SchemeId3::KEYGEN_SEED_BYTES,
        )?;
        Self::from_seed(seed.as_slice())
    }

    /// Derives an account from a 128-byte seed.
    ///
    /// # Errors
    ///
    /// Returns an engine error if the seed has the wrong length or invalid keys.
    pub fn from_seed(seed: &[u8]) -> Result<Self, SchemeError> {
        Ok(Self::from_keys(SchemeId3::keygen(seed)?))
    }

    fn from_keys((meta, master, tracking): pqsa_core::Keys<SchemeId3>) -> Self {
        debug!(scheme_id = SCHEME_ID, "scheme 3 keygen");
        Self {
            meta_address: meta,
            master_key: master,
            tracking_key: tracking,
        }
    }

    #[must_use]
    pub fn meta_address(&self) -> &StealthMetaAddress {
        &self.meta_address
    }

    #[must_use]
    pub fn master_key(&self) -> &Master {
        &self.master_key
    }

    #[must_use]
    pub fn tracking_key(&self) -> &Tracking {
        &self.tracking_key
    }

    pub fn scanner(&self) -> Result<Scanner, SchemeError> {
        bind_scanner(&self.tracking_key, &self.meta_address)
    }

    /// Derives the spend key. Fails if this account does not own the matched address.
    pub fn spend_key(&self, matched: &Match) -> Result<SpendKey, SchemeError> {
        compute_stealth_spend_key(&self.master_key, matched)
    }
}

/// Generates a stealth address and its public announcement data.
///
/// # Errors
///
/// Forwards engine errors. Returns [`EngineError::SeedRejected`] after four rejected seeds.
pub fn generate_stealth_address(
    meta_address: &StealthMetaAddress,
    rng: &mut impl CryptoRng,
) -> Result<StealthAddress, SchemeError> {
    for _ in 0..RANDOM_ANNOUNCEMENT_ATTEMPTS {
        let mut seed = Zeroizing::new([0u8; 64]);
        rng.fill_bytes(seed.as_mut());
        match generate_stealth_address_with_seed(meta_address, seed.as_slice()) {
            Ok(announcement) => return Ok(announcement),
            Err(SchemeError::Engine(EngineError::SeedRejected)) => {}
            Err(error) => return Err(error),
        }
    }
    Err(EngineError::SeedRejected.into())
}

/// Generates a stealth address from a 64-byte seed.
/// Reusing a seed repeats the ephemeral key and, for the same recipient, the address.
///
/// # Errors
///
/// Returns an engine error when the seed or meta-address is invalid.
pub fn generate_stealth_address_with_seed(
    meta_address: &StealthMetaAddress,
    seed: &[u8],
) -> Result<StealthAddress, SchemeError> {
    let generated = SchemeId3::announce(meta_address, seed)?;
    debug!(
        scheme_id = SCHEME_ID,
        stealth_address = %Address::from(generated.stealth_address),
        ciphertext_len = generated.ct().len(),
        "generated stealth address"
    );
    Ok(generated)
}

/// Creates a scanner from a tracking key and meta-address.
///
/// # Errors
///
/// Fails if the tracking key does not belong to the meta-address.
pub fn bind_scanner(
    tracking_key: &Tracking,
    meta_address: &StealthMetaAddress,
) -> Result<Scanner, SchemeError> {
    let scanner = SchemeId3::bind(tracking_key, meta_address)?;
    debug!(kem_key_len = meta_address.ek().len(), "scanner bound");
    Ok(scanner)
}

/// Returns a match for this scanner, or `None` for a foreign or invalid announcement.
#[must_use]
pub fn scan_announcement(scanner: &Scanner, announcement: &StealthAddress) -> Option<Match> {
    let Some(matched) = SchemeId3::scan(scanner, announcement) else {
        trace!(
            stealth_address = %Address::from(announcement.stealth_address),
            "skip announcement: not ours"
        );
        return None;
    };
    debug!(
        stealth_address = %Address::from(matched.stealth_address),
        "scheme 3 announcement matched"
    );
    Some(matched)
}

/// Computes the stealth private key. Fails if `master_key` does not own the matched address.
pub fn compute_stealth_spend_key(
    master_key: &Master,
    matched: &Match,
) -> Result<SpendKey, SchemeError> {
    let key = SchemeId3::spend_key(master_key, matched)?;
    debug!(
        stealth_address = %Address::from(matched.stealth_address),
        "scheme 3 spend key derived"
    );
    Ok(key)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::{
        io,
        sync::{Arc, Mutex},
    };
    use tracing::Level;

    const V3_09_SEED: &str = "1111111111111111111111111111111111111111111111111111111111111111\
         3333333333333333333333333333333333333333333333333333333333333333\
         e582b7d75e6c80b05ae392a1fc9f7153b12390fd99930368cc67a768baebc8a0\
         1cdacb8740c0b87c4a379575f187b367cbfa3b300bf591b109f79816e9cbe8f0";

    /// `pq-stealth-scheme3-public` `b576b37c` `vectors/section-2.json` V3-09 `expect.meta_address`.
    const V3_09_META: &str = "034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa\
         023c72addb4fdf09af94f0c94d7fe92a386a7e70cf8a1d85916386bb2535c7b1b1\
         28c793778741b80b02b4339f2aa4347255b099f17264e1b8cc0a2c7c2a1a79f799\
         7b907fd0496c6e6c8ad7714f5f339d75f11f625591a869be1175ae47f05fd43134\
         68232ba6957d7807b824f445ac99a0d568ab1ad54dca8249d1482e61275f52248c\
         77f61a4248753188cd1794cd0a465ec0dc4b025985c461b74e76286e4c37e77405\
         695cc9fd0654374b427a20343aec0ff1a187768273bfc4905472a1da387f14559d\
         6ce87313f6a5b6138434539f9a13684055b177e543f8b40f432abd7cc49989a50a\
         9084c660913f45a8593b17499bc4cf936c2bc1851421cb986808a0ef30afe97aab\
         5b8b8eb3f0b3506a95b91563a0e57db7231044987ef141bdab3537c316ad16f178\
         05a81f29329879a94e96157e4b7447f7d59603b21bd896cc47b7cd4e232322eb9c\
         5d2215696bcffca3a04efcc4c5d9cc39ac9a6e8700d38c244b0169e7fa1fe81b4b\
         10365e74e6a1f7f756d11acdc84043f81006d62995376c22535958feb53f78117e\
         e0f61c4c862640d06dc57a2b8be62a41a642af3bc63f6bac98bbbbff70570f37b8\
         f8d9572f2735657a6c98f96caf57a849868720b2640b8bb2732237a1f984c18872\
         d10289ce43c952c9257e06529aeb76afd127b17596fd25c5216c9cabd9b18efc50\
         e87bbb04568bb7d5c4e9288c006483af5912e19108573700bd10cd77224b80659e\
         a75aa74270b33ac4008b738bfee271e78658c8742ff13c96ad0781a03c7576ca26\
         dd58b52980ba58c0505e446afa140cdcea0490db1f9b18815d4314b2459cacc562\
         441c91f4084e5426c88e632cf7482e79907911d06473260835d7b85e7856a829ae\
         a0381707b939ce86882cc09c4448c6ae94a9c303107c5667eefb8df7763cc21189\
         a3c590c40aa51f491503a7935ec08f4fc300cbe607ed8c9100c29fbf45584b13c8\
         d780069337aec76c36ceb70373e2ab6e7b934b466f53fb32eaf040055496b8540e\
         23a2a277e534468608d5ec0f8d38cea5bbb806c1bf4f164f6ac826fe733f95461e\
         29dcc11200c0aada1b8332023eab329718ce25cc0a09555903f3578bbc863b1752\
         ca94365da556df54c3b7e05cbb7115fbc1b6c57a172c31b9906560c8fb54f3c563\
         a2256cc073243b8179b4a28d60e086cf51082ee429272996f0aabe03ba0eafd3c8\
         e7d954bd0933e2f60ed0c32cede7b820a28e48f3ca3c40913cccae2337abfc5984\
         3f08c9863325d65a4e9e15c1f46172b118b2b5eb0f1d5158a00134f27b085488c3\
         a0621fe4e5678698250fb74ee5152e3e35a66544a05d279ea99131fbc15165060b\
         90f88eeb7b20892a4de4cb1683495bd7da037966b47cc040f1764c5deb06b5499d\
         4267391cebbb47f734d8539e39528436a1858182854bf20b1f93279afb706464c6\
         5ccc5ae099b37cc03556c26abf4c3f8b9ba3a936707211a49a59b268f5284f7970\
         c77612719450377417428c4ba47c9ca115cf95304c4759c5d8859b44985c06a6c9\
         24689237ba320d610960d61c53e85431789e67a40113f167ff93429c264f6cabc9\
         5448c903437d39a6577be0cf0012852aa476351a9046a110a1a625a3d74c910b78\
         bce9cfca735e4f91b8a4c57dbe489e849446098aacf73070aee638fcc8896473d3\
         c159d3afb4b687b40dfbf371a9c2644b605187b71a14bc4c8678fe8247";

    fn unhex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        hex::decode(s).unwrap()
    }

    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn captured(body: impl FnOnce()) -> String {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = Arc::clone(&bytes);
        // Multiple dispatches force tracing to check the thread's current subscriber.
        let _inactive = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(Level::TRACE)
            .with_writer(move || LogWriter(Arc::clone(&writer)))
            .finish();
        let capture = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&capture, body);
        String::from_utf8(bytes.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn keygen_master_derivation() {
        let derived = Scheme3Account::from_keygen_master(&[0xa5; 32]).unwrap();

        assert_eq!(derived.keygen_index, 0);
        // Scheme 3 seed from pqsa-core's keygen_seed_golden test.
        let seed = unhex(concat!(
            "42bd3c7fd29ccc42e9f8a655995fbfd4699b7f53daf62c9142f591908ccbb03d",
            "a9c53cfc18d95955ed3222013bce036a8e6d2fe790d614f3ab86b8cb187c4b89",
            "447412f0c3d2978d6b1c1b1830907c82c214f889af40478f2b84efe79e9d15e1",
            "a4cb056acf428e47138a6520c4494ec7b3c244082e7d31f44ad1f24327a2fd6c"
        ));
        let expected = Scheme3Account::from_seed(&seed).unwrap();
        assert_eq!(
            SchemeId3::meta_to_bytes(derived.account.meta_address()),
            SchemeId3::meta_to_bytes(expected.meta_address())
        );
        let restored =
            Scheme3Account::from_keygen_master_at(&[0xa5; 32], derived.keygen_index).unwrap();
        assert_eq!(
            SchemeId3::meta_to_bytes(restored.meta_address()),
            SchemeId3::meta_to_bytes(derived.account.meta_address())
        );
        let next = Scheme3Account::from_keygen_master_at(&[0xa5; 32], 1).unwrap();
        assert_ne!(
            SchemeId3::meta_to_bytes(next.meta_address()),
            SchemeId3::meta_to_bytes(restored.meta_address())
        );
    }

    #[test]
    fn keygen_master_rejects_wrong_lengths() {
        assert_eq!(
            Scheme3Account::from_keygen_master(&[7; 31]).unwrap_err(),
            SchemeError::Engine(EngineError::Malformed)
        );
        assert_eq!(
            Scheme3Account::from_keygen_master_at(&[7; 33], 0).unwrap_err(),
            SchemeError::Engine(EngineError::Malformed)
        );
    }

    #[test]
    fn v3_09_keygen_matches_the_published_meta_address() {
        let seed = unhex(V3_09_SEED);
        let account = Scheme3Account::from_seed(&seed).unwrap();
        assert_eq!(
            SchemeId3::meta_to_bytes(account.meta_address()),
            unhex(V3_09_META)
        );
    }

    #[test]
    fn invalid_account_seed() {
        for seed in [&[0x11; 96][..], &[0x11; 127][..]] {
            assert_eq!(
                Scheme3Account::from_seed(seed).unwrap_err(),
                SchemeError::Engine(EngineError::Malformed)
            );
        }
        let mut seed = unhex(V3_09_SEED);
        seed[..32].fill(0);
        assert_eq!(
            Scheme3Account::from_seed(&seed).unwrap_err(),
            SchemeError::Engine(EngineError::NoValidScalar)
        );
    }

    #[test]
    fn foreign_account() {
        let account = Scheme3Account::from_keygen_master(&[7; 32])
            .unwrap()
            .account;
        let other = Scheme3Account::from_keygen_master(&[8; 32])
            .unwrap()
            .account;
        let announcement =
            generate_stealth_address_with_seed(account.meta_address(), &[0x44; 64]).unwrap();
        let scanner = account.scanner().unwrap();
        let matched = scan_announcement(&scanner, &announcement).unwrap();
        assert_eq!(
            Address::from(matched.stealth_address),
            Address::from(announcement.stealth_address)
        );
        assert_eq!(
            other.spend_key(&matched).unwrap_err(),
            SchemeError::Engine(EngineError::MasterKeyMismatch)
        );
        assert!(scan_announcement(&other.scanner().unwrap(), &announcement).is_none());

        let mut wrong_tag = announcement.clone();
        wrong_tag.view_tag[0] ^= 0xff;
        assert!(scan_announcement(&scanner, &wrong_tag).is_none());

        let mut wrong_address = announcement;
        wrong_address.stealth_address = Address::ZERO.into_array();
        assert!(scan_announcement(&scanner, &wrong_address).is_none());
    }

    #[test]
    fn announcement_seed() {
        let account = Scheme3Account::from_seed(&unhex(V3_09_SEED)).unwrap();
        let seed = [0x44; 64];
        let first = generate_stealth_address_with_seed(account.meta_address(), &seed).unwrap();
        let second = generate_stealth_address_with_seed(account.meta_address(), &seed).unwrap();
        assert_eq!(
            SchemeId3::announcement_to_bytes(&first),
            SchemeId3::announcement_to_bytes(&second)
        );
        let mut rejected = seed;
        rejected[..32].fill(0);
        assert_eq!(
            generate_stealth_address_with_seed(account.meta_address(), &rejected).unwrap_err(),
            SchemeError::Engine(EngineError::SeedRejected)
        );
        assert_eq!(
            generate_stealth_address_with_seed(account.meta_address(), &seed[..63]).unwrap_err(),
            SchemeError::Engine(EngineError::Malformed)
        );
    }

    struct SeedRng {
        bytes: Vec<u8>,
        consumed: usize,
    }

    impl rand::TryRng for SeedRng {
        type Error = core::convert::Infallible;

        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            let mut bytes = [0u8; 4];
            self.try_fill_bytes(&mut bytes)?;
            Ok(u32::from_le_bytes(bytes))
        }

        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            let mut bytes = [0u8; 8];
            self.try_fill_bytes(&mut bytes)?;
            Ok(u64::from_le_bytes(bytes))
        }

        fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
            let end = self.consumed + dst.len();
            dst.copy_from_slice(&self.bytes[self.consumed..end]);
            self.consumed = end;
            Ok(())
        }
    }

    impl rand::TryCryptoRng for SeedRng {}

    #[test]
    fn account_seed_uses_rng_bytes() {
        let bytes: Vec<u8> = (0u8..=255).collect();
        let mut rng = SeedRng {
            bytes: bytes.clone(),
            consumed: 0,
        };
        for expected in bytes.chunks_exact(128) {
            let seed = AccountSeed::generate(&mut rng);
            assert_eq!(seed.expose_secret().as_slice(), expected);
        }
        assert_eq!(rng.consumed, bytes.len());
    }

    #[test]
    fn retries_rejected_seed() {
        let account = Scheme3Account::from_keygen_master(&[7; 32])
            .unwrap()
            .account;
        let mut rng = SeedRng {
            bytes: [vec![0; 64], vec![0x44; 64]].concat(),
            consumed: 0,
        };
        let generated = generate_stealth_address(account.meta_address(), &mut rng).unwrap();
        let expected =
            generate_stealth_address_with_seed(account.meta_address(), &[0x44; 64]).unwrap();
        assert_eq!(
            SchemeId3::announcement_to_bytes(&generated),
            SchemeId3::announcement_to_bytes(&expected)
        );
        assert_eq!(rng.consumed, 128);
    }

    #[test]
    fn seed_retry_limit() {
        let account = Scheme3Account::from_keygen_master(&[7; 32])
            .unwrap()
            .account;
        let mut rng = SeedRng {
            bytes: vec![0; 5 * 64],
            consumed: 0,
        };
        assert_eq!(
            generate_stealth_address(account.meta_address(), &mut rng).unwrap_err(),
            SchemeError::Engine(EngineError::SeedRejected)
        );
        assert_eq!(rng.consumed, 256);
    }

    #[test]
    fn invalid_kem_key() {
        let account = Scheme3Account::from_keygen_master(&[7; 32])
            .unwrap()
            .account;
        let meta = account.meta_address();
        let recipient =
            StealthMetaAddress::new(meta.spending, meta.viewing_ec, vec![0xff; meta.ek().len()])
                .unwrap();
        let mut rng = SeedRng {
            bytes: vec![0x44; 2 * 64],
            consumed: 0,
        };
        assert_eq!(
            generate_stealth_address(&recipient, &mut rng).unwrap_err(),
            SchemeError::Engine(EngineError::Kem)
        );
        assert_eq!(rng.consumed, 64);
    }

    #[test]
    fn seed_debug_is_redacted() {
        let account_seed = AccountSeed::from_bytes([0x7f; 128]);
        assert_eq!(format!("{account_seed:?}"), "AccountSeed([REDACTED])");
    }

    #[test]
    fn secrets_are_not_logged() {
        let seed = unhex(V3_09_SEED);
        let mut secret_renderings = Vec::new();
        let logged = captured(|| {
            let account = Scheme3Account::from_seed(&seed).unwrap();
            let account_debug = format!("{account:?}");
            let master = account.master_key();
            for secret in [
                master.spending_seed.as_slice(),
                master.viewing_ec_seed.as_ref().unwrap().as_slice(),
                master.kem_seed.as_slice(),
            ] {
                secret_renderings.push(hex::encode(secret));
                secret_renderings.push(format!("{secret:?}"));
                assert!(!account_debug.contains(&hex::encode(secret)));
                assert!(!account_debug.contains(&format!("{secret:?}")));
            }

            let generated =
                generate_stealth_address_with_seed(account.meta_address(), &[0x44; 64]).unwrap();
            let scanner = account.scanner().unwrap();
            let matched = scan_announcement(&scanner, &generated).unwrap();
            let key = account.spend_key(&matched).unwrap();

            for secret in [
                matched.shared_secret.as_slice(),
                key.expose_secret().as_slice(),
            ] {
                secret_renderings.push(hex::encode(secret));
                secret_renderings.push(format!("{secret:?}"));
            }
        });
        for message in [
            "scheme 3 keygen",
            "generated stealth address",
            "scanner bound",
            "scheme 3 announcement matched",
            "scheme 3 spend key derived",
        ] {
            assert!(
                logged.contains(message),
                "missing captured event: {message}"
            );
        }
        assert!(!logged.contains(&hex::encode(&seed)));
        assert!(!logged.contains(&hex::encode([0x44; 64])));
        for secret in secret_renderings {
            assert!(!logged.contains(&secret));
        }
    }
}
