use alloy::primitives::Address;
use kohaku_tornadocash::{PaymasterInfo as CorePaymasterInfo, Pool as CorePool};
use serde::{Deserialize, Serialize};
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::{
    asset::Asset,
    bigint::{JsU64, JsU128},
    hex::Hex,
    note_string::NoteString,
};

/// A tornadocash pool.
///
/// See [`kohaku_tornadocash::Pool`] for more details.
#[wasm_bindgen]
pub struct Pool {
    pub(crate) inner: CorePool,
}

/// Privacy-paymaster metadata for a pool.
///
/// See [`kohaku_tornadocash::PaymasterInfo`] for more details.
#[derive(Serialize, Deserialize, Tsify)]
pub struct PaymasterInfo {
    pub address: Hex,
    pub adapter: Hex,
}

impl TryFrom<PaymasterInfo> for CorePaymasterInfo {
    type Error = crate::hex::HexError;

    fn try_from(value: PaymasterInfo) -> Result<Self, Self::Error> {
        Ok(Self {
            address: value.address.try_into()?,
            adapter: value.adapter.try_into()?,
        })
    }
}

impl From<CorePaymasterInfo> for PaymasterInfo {
    fn from(value: CorePaymasterInfo) -> Self {
        Self {
            address: value.address.into(),
            adapter: value.adapter.into(),
        }
    }
}

#[wasm_bindgen]
impl Pool {
    /// Construct an independently owned pool, borrowing and cloning the asset metadata.
    ///
    /// The asset remains usable and can be freed independently of the pool.
    ///
    /// # Errors
    ///
    /// Throws if `chainId`/`deployedBlock` are not u64 bigints, `amountWei` is not a
    /// u128 bigint, or any supplied address cannot be decoded into 20 bytes.
    #[wasm_bindgen(constructor)]
    pub fn new(
        chain_id: JsU64,
        address: &Ts<Hex>,
        asset: &Asset,
        amount_wei: JsU128,
        deployed_block: JsU64,
        paymaster: Option<Ts<PaymasterInfo>>,
    ) -> Result<Self, JsError> {
        let chain_id = u64::try_from(chain_id)?;
        let address = Address::try_from(address.to_rust()?)?;
        let amount_wei = u128::try_from(amount_wei)?;
        let deployed_block = u64::try_from(deployed_block)?;
        let paymaster = paymaster
            .map(|value| -> Result<_, JsError> {
                Ok(CorePaymasterInfo::try_from(value.to_rust()?)?)
            })
            .transpose()?;

        Ok(Self {
            inner: CorePool {
                chain_id,
                address,
                asset: asset.inner.clone(),
                amount_wei,
                deployed_block,
                paymaster,
            },
        })
    }

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

    /// Return plain paymaster metadata with checksummed addresses, or undefined.
    ///
    /// The returned JavaScript object requires no `free()` call.
    #[wasm_bindgen(getter)]
    pub fn paymaster(&self) -> Result<Option<Ts<PaymasterInfo>>, JsError> {
        self.inner
            .paymaster
            .map(|value| PaymasterInfo::from(value).into_ts())
            .transpose()
            .map_err(Into::into)
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
    use alloy::primitives::Address;
    use tsify::{Ts, Tsify};
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{CorePaymasterInfo, CorePool, PaymasterInfo, Pool};
    use crate::{asset::Asset, hex::Hex};

    fn address() -> Ts<Hex> {
        Hex::from(Address::with_last_byte(0xab)).into_ts().unwrap()
    }

    #[wasm_bindgen_test]
    fn custom_pool_maps_exact_fields_and_owns_borrowed_asset_metadata() {
        let token_address = Hex::from(Address::with_last_byte(0xcd)).into_ts().unwrap();
        let asset = Asset::erc20(&token_address, " Tkn ".to_owned(), 255_u8.into()).unwrap();
        let expected_asset = asset.inner.clone();
        let pool = Pool::new(
            u64::MAX.into(),
            &address(),
            &asset,
            u128::MAX.into(),
            9_007_199_254_740_993_u64.into(),
            None,
        )
        .unwrap();
        drop(asset);

        assert_eq!(pool.inner.chain_id, u64::MAX);
        assert_eq!(pool.inner.address, Address::with_last_byte(0xab));
        assert_eq!(pool.inner.asset, expected_asset);
        assert_eq!(pool.inner.amount_wei, u128::MAX);
        assert_eq!(pool.inner.deployed_block, 9_007_199_254_740_993);
        let paymaster: JsValue = pool.paymaster().unwrap().into();
        assert!(paymaster.is_undefined());
    }

    #[wasm_bindgen_test]
    fn custom_paymaster_maps_input_and_output_fields() {
        let expected = CorePaymasterInfo {
            address: Address::with_last_byte(0xab),
            adapter: Address::with_last_byte(0xcd),
        };
        let input = PaymasterInfo {
            address: Hex::from(expected.address),
            adapter: Hex::from(expected.adapter),
        }
        .into_ts()
        .unwrap();
        let pool = Pool::new(
            0_u64.into(),
            &address(),
            &Asset::eth(),
            0_u128.into(),
            0_u64.into(),
            Some(input),
        )
        .unwrap();
        assert_eq!(pool.inner.paymaster, Some(expected));
        let output = pool.paymaster().unwrap().unwrap().to_rust().unwrap();
        assert_eq!(Address::try_from(output.address).unwrap(), expected.address);
        assert_eq!(Address::try_from(output.adapter).unwrap(), expected.adapter);
    }

    #[wasm_bindgen_test]
    fn pool_address_is_checksummed_hex() {
        let pool = Pool {
            inner: CorePool::ETHEREUM_ETHER_01,
        };
        let expected = pool.inner.address.to_string();

        let address: JsValue = pool.address().unwrap().into();
        assert_eq!(address.as_string().unwrap(), expected);
    }
}
