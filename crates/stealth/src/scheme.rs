pub(crate) const SCHEME_ID: u64 = SchemeId3::SCHEME_ID;

use pqsa_core::{ExportableSpendKey, StealthScheme};
use pqsa_per_payment::SchemeId3;
use thiserror::Error;
use tracing::{debug, trace, warn};
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SchemeError {
    #[error("malformed input")]
    Malformed,
    #[error("no valid secp256k1 scalar")]
    NoValidScalar,
    #[error("spending seed appears in delegated scan material")]
    SpendingKeyDelegated,
    #[error("sender counter exhausted")]
    CounterExhausted,
    #[error("key encapsulation failed")]
    Kem,
    #[error("announce seed rejected")]
    SeedRejected,
    #[error("tracking key does not match the meta-address")]
    TrackingKeyMismatch,
    #[error("master key does not control the stealth address")]
    MasterKeyMismatch,
    #[error("address mapping is not closed")]
    AddressMappingOpen,
}

impl From<pqsa_core::Error> for SchemeError {
    fn from(err: pqsa_core::Error) -> Self {
        match err {
            pqsa_core::Error::Malformed => Self::Malformed,
            pqsa_core::Error::NoValidScalar => Self::NoValidScalar,
            pqsa_core::Error::SpendingKeyDelegated => Self::SpendingKeyDelegated,
            pqsa_core::Error::CounterExhausted => Self::CounterExhausted,
            pqsa_core::Error::Kem => Self::Kem,
            pqsa_core::Error::SeedRejected => Self::SeedRejected,
            pqsa_core::Error::TrackingKeyMismatch => Self::TrackingKeyMismatch,
            pqsa_core::Error::MasterKeyMismatch => Self::MasterKeyMismatch,
            pqsa_core::Error::AddressMappingOpen => Self::AddressMappingOpen,
        }
    }
}

pub(crate) struct Master(pqsa_per_payment::Master);

impl Drop for Master {
    fn drop(&mut self) {
        self.0.spending_seed.zeroize();
        if let Some(seed) = &mut self.0.viewing_ec_seed {
            seed.zeroize();
        }
        self.0.kem_seed.zeroize();
    }
}

impl core::fmt::Debug for Master {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Master([REDACTED])")
    }
}

/// Scan material. May be handed to a scanner.
pub(crate) struct Tracking(pqsa_per_payment::Tracking);

impl Drop for Tracking {
    fn drop(&mut self) {
        if let Some(seed) = &mut self.0.viewing_ec_seed {
            seed.zeroize();
        }
        self.0.kem_seed.zeroize();
    }
}

impl core::fmt::Debug for Tracking {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Tracking([REDACTED])")
    }
}

/// Output of [`keygen`].
#[derive(Debug)]
pub(crate) struct Keys {
    /// ERC-6538 blob, 1 250 bytes on success.
    pub meta_address: Vec<u8>,
    pub master: Master,
    pub tracking: Tracking,
}

/// `SchemeId3::bind` result. Constructed only by [`bind`].
pub(crate) struct Scanner(pqsa_per_payment::Scanner);

impl core::fmt::Debug for Scanner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Scanner([REDACTED])")
    }
}

pub(crate) struct Match {
    pub stealth_address: [u8; 20],
    matched: pqsa_per_payment::Match,
}

impl Drop for Match {
    fn drop(&mut self) {
        self.matched.shared_secret.zeroize();
    }
}

impl core::fmt::Debug for Match {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Match")
            .field("stealth_address", &self.stealth_address)
            .finish_non_exhaustive()
    }
}

/// ERC-5564 `announce` arguments for scheme 3.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Announcement {
    pub scheme_id: u64,
    pub stealth_address: [u8; 20],
    pub ephemeral_pub_key: Vec<u8>,
    pub metadata: Vec<u8>,
}

impl core::fmt::Debug for Announcement {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Announcement")
            .field("scheme_id", &self.scheme_id)
            .field("stealth_address", &self.stealth_address)
            .field("ephemeral_pub_key_len", &self.ephemeral_pub_key.len())
            .field("metadata_len", &self.metadata.len())
            .finish()
    }
}

pub(crate) fn keygen(seed: &[u8]) -> Result<Keys, SchemeError> {
    let (meta, master, tracking) = SchemeId3::keygen(seed)?;
    let meta_address = SchemeId3::meta_to_bytes(&meta);
    debug!(
        scheme_id = SchemeId3::SCHEME_ID,
        meta_address_len = meta_address.len(),
        "scheme 3 keygen"
    );
    Ok(Keys {
        meta_address,
        master: Master(master),
        tracking: Tracking(tracking),
    })
}

pub(crate) fn bind(tracking: &Tracking, meta_address: &[u8]) -> Result<Scanner, SchemeError> {
    let Some(meta) = SchemeId3::meta_from_bytes(meta_address) else {
        warn!(
            meta_address_len = meta_address.len(),
            "meta-address is not scheme 3"
        );
        return Err(SchemeError::Malformed);
    };
    let scanner = SchemeId3::bind(&tracking.0, &meta)?;
    debug!(meta_address_len = meta_address.len(), "scanner bound");
    Ok(Scanner(scanner))
}

pub(crate) fn announce_with_seed(
    meta_address: &[u8],
    seed: &[u8],
) -> Result<Announcement, SchemeError> {
    let Some(meta) = SchemeId3::meta_from_bytes(meta_address) else {
        return Err(SchemeError::Malformed);
    };
    let announcement = SchemeId3::announce(&meta, seed)?;
    let (stealth_address, ephemeral_pub_key, metadata) =
        SchemeId3::announcement_to_bytes(&announcement);
    log_announcement(&stealth_address, ephemeral_pub_key.len(), metadata.len());
    Ok(Announcement {
        scheme_id: SchemeId3::SCHEME_ID,
        stealth_address,
        ephemeral_pub_key,
        metadata,
    })
}

fn log_announcement(stealth_address: &[u8; 20], ephemeral_pub_key_len: usize, metadata_len: usize) {
    debug!(
        scheme_id = SchemeId3::SCHEME_ID,
        stealth_address = %alloy_address(stealth_address),
        ephemeral_pub_key_len,
        metadata_len,
        "scheme 3 announcement"
    );
}

#[must_use]
pub(crate) fn check(
    scanner: &Scanner,
    scheme_id: u64,
    stealth_address: &[u8; 20],
    ephemeral_pub_key: &[u8],
    metadata: &[u8],
) -> Option<Match> {
    if scheme_id != SchemeId3::SCHEME_ID {
        trace!(scheme_id, "skip announcement: scheme id");
        return None;
    }
    let Some(announcement) =
        SchemeId3::announcement_from_bytes(stealth_address, ephemeral_pub_key, metadata)
    else {
        trace!(
            ephemeral_pub_key_len = ephemeral_pub_key.len(),
            metadata_len = metadata.len(),
            "skip announcement: shape"
        );
        return None;
    };
    let Some(matched) = SchemeId3::scan(&scanner.0, &announcement) else {
        trace!(
            stealth_address = %alloy_address(stealth_address),
            "skip announcement: not ours"
        );
        return None;
    };
    debug!(
        stealth_address = %alloy_address(&matched.stealth_address),
        "scheme 3 payment matched"
    );
    Some(Match {
        stealth_address: matched.stealth_address,
        matched,
    })
}

pub(crate) fn spend_key(
    master: &Master,
    found: &Match,
) -> Result<Zeroizing<[u8; 32]>, SchemeError> {
    let scalar = Zeroizing::new(SchemeId3::spend_key(&master.0, &found.matched)?);
    let bytes = SchemeId3::spend_key_bytes(&scalar);
    let out: [u8; 32] = bytes.try_into().map_err(|_| SchemeError::Malformed)?;
    debug!(
        stealth_address = %alloy_address(&found.stealth_address),
        "scheme 3 spend key derived"
    );
    Ok(Zeroizing::new(out))
}

#[must_use]
pub(crate) fn meta_address_is_valid(meta_address: &[u8]) -> bool {
    SchemeId3::meta_from_bytes(meta_address).is_some()
}

pub(crate) fn announcement_is_valid(
    stealth_address: &[u8; 20],
    ephemeral_pub_key: &[u8],
    metadata: &[u8],
) -> bool {
    SchemeId3::announcement_from_bytes(stealth_address, ephemeral_pub_key, metadata).is_some()
}

fn alloy_address(bytes: &[u8; 20]) -> alloy::primitives::Address {
    alloy::primitives::Address::from(*bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use tracing::Level;
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;

    const V3_09_SEED: &str = "1111111111111111111111111111111111111111111111111111111111111111\
         3333333333333333333333333333333333333333333333333333333333333333\
         e582b7d75e6c80b05ae392a1fc9f7153b12390fd99930368cc67a768baebc8a0\
         1cdacb8740c0b87c4a379575f187b367cbfa3b300bf591b109f79816e9cbe8f0";

    /// `pq-stealth-scheme3-public` `5fe8d0fd` `vectors/section-2_9.json` V3-09 `expect.meta_address`.
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

    struct Capture(Arc<Mutex<Vec<String>>>);

    impl<S> Layer<S> for Capture
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = MsgVisitor(String::new());
            event.record(&mut visitor);
            self.0.lock().unwrap().push(visitor.0);
        }
    }

    struct MsgVisitor(String);

    impl tracing::field::Visit for MsgVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn core::fmt::Debug) {
            use std::fmt::Write;
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }

    fn captured(body: impl FnOnce()) -> String {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let layer = Capture(Arc::clone(&lines)).with_filter(
            tracing_subscriber::filter::LevelFilter::from_level(Level::TRACE),
        );
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), body);
        lines.lock().unwrap().join("\n")
    }

    #[test]
    fn v3_09_keygen_matches_the_published_meta_address() {
        let seed = unhex(V3_09_SEED);
        let registered = keygen(&seed).unwrap();
        assert_eq!(registered.meta_address, unhex(V3_09_META));
        assert_eq!(registered.meta_address.len(), 1250);
        let again = keygen(&seed).unwrap();
        assert_eq!(registered.meta_address, again.meta_address);
    }

    #[test]
    fn keygen_rejects_a_zero_spending_scalar_instead_of_reducing_it() {
        let mut seed = unhex(V3_09_SEED);
        seed[..32].fill(0);
        assert_eq!(keygen(&seed).unwrap_err(), SchemeError::NoValidScalar);

        let mut order = unhex(V3_09_SEED);
        order[..32].copy_from_slice(&unhex(
            "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
        ));
        assert_eq!(keygen(&order).unwrap_err(), SchemeError::NoValidScalar);

        let mut below = unhex(V3_09_SEED);
        below[..32].copy_from_slice(&unhex(
            "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364140",
        ));
        assert!(keygen(&below).is_ok());
    }

    #[test]
    fn keygen_rejects_the_wrong_seed_length() {
        assert_eq!(keygen(&[0x11; 96]).unwrap_err(), SchemeError::Malformed);
        assert_eq!(keygen(&[0x11; 127]).unwrap_err(), SchemeError::Malformed);
    }

    #[test]
    fn announce_with_seed_repeats_and_does_not_log_the_seed() {
        let registered = keygen(&unhex(V3_09_SEED)).unwrap();
        let seed = [0x44u8; 64];
        let seed_hex = hex::encode(seed);
        let mut first = None;
        let logged = captured(|| {
            first = Some(announce_with_seed(&registered.meta_address, &seed).unwrap());
        });
        let first = first.unwrap();
        let second = announce_with_seed(&registered.meta_address, &seed).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.ephemeral_pub_key.len(), 33);
        assert_eq!(first.metadata.len(), 1089);
        assert!(!logged.contains(&seed_hex));

        let mut rejected = seed;
        rejected[..32].fill(0);
        assert_eq!(
            announce_with_seed(&registered.meta_address, &rejected).unwrap_err(),
            SchemeError::SeedRejected
        );
        assert_eq!(
            announce_with_seed(&registered.meta_address, &seed[..63]).unwrap_err(),
            SchemeError::Malformed
        );
    }

    #[test]
    fn round_trip_and_skips() {
        let registered = keygen(&unhex(V3_09_SEED)).unwrap();
        let wire = announce_with_seed(&registered.meta_address, &[0x42; 64]).unwrap();
        assert_eq!(wire.scheme_id, 3);
        assert_eq!(wire.ephemeral_pub_key.len(), 33);
        assert_eq!(wire.metadata.len(), 1089);

        let scanner = bind(&registered.tracking, &registered.meta_address).unwrap();
        let found = check(
            &scanner,
            wire.scheme_id,
            &wire.stealth_address,
            &wire.ephemeral_pub_key,
            &wire.metadata,
        )
        .unwrap();
        assert_eq!(found.stealth_address, wire.stealth_address);
        let scalar = spend_key(&registered.master, &found).unwrap();
        assert_eq!(scalar.len(), 32);
        assert_ne!(*scalar, [0; 32]);

        assert!(
            check(
                &scanner,
                4,
                &wire.stealth_address,
                &wire.ephemeral_pub_key,
                &wire.metadata,
            )
            .is_none()
        );

        let mut flipped = wire.metadata.clone();
        flipped[0] ^= 0xff;
        assert!(
            check(
                &scanner,
                3,
                &wire.stealth_address,
                &wire.ephemeral_pub_key,
                &flipped,
            )
            .is_none()
        );
    }

    #[test]
    fn debug_and_logs_do_not_contain_the_seed() {
        let seed = unhex(V3_09_SEED);
        let seed_hex = hex::encode(&seed);
        let (rendered, master, tracking, joined) = {
            let mut registered = None;
            let joined = captured(|| {
                registered = Some(keygen(&seed).unwrap());
            });
            let registered = registered.unwrap();
            (
                format!("{registered:?}"),
                format!("{:?}", registered.master),
                format!("{:?}", registered.tracking),
                joined,
            )
        };
        assert!(!rendered.contains(&seed_hex));
        assert!(!master.contains("11111111"));
        assert!(!tracking.contains("33333333"));
        assert!(!joined.contains("1111111111111111"));
        assert!(!joined.contains(&seed_hex));
    }
}
