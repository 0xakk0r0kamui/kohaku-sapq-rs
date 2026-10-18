use kohaku_tornadocash::Pool as CorePool;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::{asset::Asset, hex::Hex, note_string::NoteString};

/// A tornadocash pool.
///
/// See [`kohaku_tornadocash::Pool`] for more details.
#[wasm_bindgen]
pub struct Pool {
    inner: CorePool,
}

#[wasm_bindgen]
impl Pool {
    /// Return independently owned wrappers for every entry in the core pool catalog.
    #[must_use]
    pub fn known() -> Vec<Self> {
        CorePool::POOLS
            .iter()
            .cloned()
            .map(|inner| Self { inner })
            .collect()
    }

    /// Find a known pool from the note's hints without consuming the note.
    ///
    /// Return an independently owned wrapper, or undefined if no pool matches.
    #[wasm_bindgen(js_name = fromNote)]
    #[must_use]
    pub fn from_note(note: &NoteString) -> Option<Self> {
        CorePool::from_note(&note.inner).map(|inner| Self { inner })
    }

    /// Find a known pool by its contract address.
    ///
    /// Return an independently owned wrapper, or undefined if no pool matches.
    ///
    /// # Errors
    ///
    /// Throws if the input cannot be decoded into a 20-byte address.
    #[wasm_bindgen(js_name = fromAddress)]
    pub fn from_address(address: &Ts<Hex>) -> Result<Option<Self>, JsError> {
        let address = address.to_rust()?.try_into()?;
        Ok(CorePool::from_address(address).map(|inner| Self { inner }))
    }

    /// Return the pool's chain ID.
    #[wasm_bindgen(getter, js_name = chainId)]
    #[must_use]
    pub fn chain_id(&self) -> u64 {
        self.inner.chain_id
    }

    /// Return the pool contract address as checksummed hex.
    #[wasm_bindgen(getter)]
    pub fn address(&self) -> Result<Ts<Hex>, JsError> {
        Ok(Hex::from(self.inner.address).into_ts()?)
    }

    /// Return an independently owned asset wrapper.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn asset(&self) -> Asset {
        Asset {
            inner: self.inner.asset.clone(),
        }
    }

    /// Return the fixed deposit amount in the asset's base units.
    #[wasm_bindgen(getter, js_name = amountWei)]
    #[must_use]
    pub fn amount_wei(&self) -> u128 {
        self.inner.amount_wei
    }

    /// Return the block at which the pool was deployed.
    #[wasm_bindgen(getter, js_name = deployedBlock)]
    #[must_use]
    pub fn deployed_block(&self) -> u64 {
        self.inner.deployed_block
    }

    /// Return the pool ID using the core's representation.
    #[must_use]
    pub fn id(&self) -> String {
        self.inner.id()
    }

    /// Return the pool asset symbol.
    #[must_use]
    pub fn symbol(&self) -> String {
        self.inner.symbol().to_owned()
    }

    /// Return the fixed deposit amount as decimal text using the core's formatting.
    #[must_use]
    pub fn amount(&self) -> String {
        self.inner.amount()
    }

    /// Format the pool using the core's display representation.
    #[wasm_bindgen(js_name = toString)]
    #[must_use]
    pub fn format(&self) -> String {
        self.inner.to_string()
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use tsify::Tsify;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{CorePool, Pool};
    use crate::hex::Hex;

    #[wasm_bindgen_test]
    fn pool_address_is_checksummed_hex() {
        let pool = Pool {
            inner: CorePool::ETHEREUM_ETHER_01,
        };
        let expected = pool.inner.address.to_string();

        let address: JsValue = pool.address().unwrap().into();
        assert_eq!(address.as_string().unwrap(), expected);
    }

    #[wasm_bindgen_test]
    fn address_lookup_decodes_hex_and_rejects_invalid_length() {
        let core = CorePool::ETHEREUM_ETHER_01;
        let address = Hex::from(core.address).into_ts().unwrap();
        let pool = Pool::from_address(&address).unwrap().unwrap();
        assert_eq!(pool.inner.address, core.address);

        let short_address = Hex::from([0xab].as_slice()).into_ts().unwrap();
        assert!(Pool::from_address(&short_address).is_err());
    }
}
