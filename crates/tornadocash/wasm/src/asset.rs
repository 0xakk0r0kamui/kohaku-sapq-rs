use std::borrow::Cow;

use alloy::primitives::Address;
use kohaku_tornadocash::Asset as CoreAsset;
use serde::{Deserialize, Serialize};
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::{bigint::JsU8, hex::Hex};

/// Represents an asset in a tornadocash pool.
///
/// See [`kohaku_tornadocash::Asset`] for more details.
#[wasm_bindgen]
pub struct Asset {
    pub(crate) inner: CoreAsset,
}

/// Represents the kind of a tornadocash asset.
#[derive(Serialize, Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub enum AssetKind {
    Native,
    Erc20,
}

#[wasm_bindgen]
impl Asset {
    /// Construct an independently owned native asset, preserving the supplied symbol.
    ///
    /// # Errors
    ///
    /// Throws if decimals is not a finite integer JavaScript number in 0..=255.
    pub fn native(symbol: String, decimals: JsU8) -> Result<Self, JsError> {
        Ok(Self {
            inner: CoreAsset::Native {
                symbol: Cow::Owned(symbol),
                decimals: u8::try_from(decimals)?,
            },
        })
    }

    /// Construct an independently owned ERC20 asset, preserving the supplied symbol.
    ///
    /// # Errors
    ///
    /// Throws if the address cannot be decoded into 20 bytes or decimals is not a
    /// finite integer JavaScript number in 0..=255.
    pub fn erc20(address: &Ts<Hex>, symbol: String, decimals: JsU8) -> Result<Self, JsError> {
        let address = Address::try_from(address.to_rust()?)?;

        Ok(Self {
            inner: CoreAsset::Erc20 {
                address,
                symbol: Cow::Owned(symbol),
                decimals: u8::try_from(decimals)?,
            },
        })
    }

    /// Return an independently owned wrapper for the core's native ETH asset.
    #[must_use]
    pub fn eth() -> Self {
        Self {
            inner: CoreAsset::ETH,
        }
    }

    /// Return an independently owned wrapper for the core's native MATIC asset.
    #[must_use]
    pub fn matic() -> Self {
        Self {
            inner: CoreAsset::MATIC,
        }
    }

    /// Return an independently owned wrapper for the core's Ethereum DAI token.
    #[wasm_bindgen(js_name = ethereumDai)]
    #[must_use]
    pub fn ethereum_dai() -> Self {
        Self {
            inner: CoreAsset::ETHEREUM_DAI,
        }
    }

    /// Return the asset variant: `native` or `erc20`.
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> Result<Ts<AssetKind>, JsError> {
        Ok(match &self.inner {
            CoreAsset::Native { .. } => AssetKind::Native,
            CoreAsset::Erc20 { .. } => AssetKind::Erc20,
        }
        .into_ts()?)
    }

    /// Return the asset symbol.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn symbol(&self) -> String {
        self.inner.symbol().to_owned()
    }

    /// Return the number of decimal places used by the asset.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn decimals(&self) -> u8 {
        self.inner.decimals()
    }

    /// Return the ERC20 token address as checksummed hex, or undefined for a native asset.
    #[wasm_bindgen(getter)]
    pub fn address(&self) -> Result<Option<Ts<Hex>>, JsError> {
        match &self.inner {
            CoreAsset::Native { .. } => Ok(None),
            CoreAsset::Erc20 { address, .. } => Ok(Some(Hex::from(*address).into_ts()?)),
        }
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use std::borrow::Cow;

    use alloy::primitives::Address;
    use kohaku_tornadocash::Asset as CoreAsset;
    use tsify::Ts;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::Asset;

    #[wasm_bindgen_test]
    fn custom_native_maps_inputs_to_owned_core_asset() {
        let asset = Asset::native(" XyZ ".to_owned(), 6_u8.into()).unwrap();

        let CoreAsset::Native { symbol, decimals } = asset.inner else {
            panic!("expected native asset");
        };
        assert!(matches!(&symbol, Cow::Owned(_)));
        assert_eq!(symbol, " XyZ ");
        assert_eq!(decimals, 6);
    }

    #[wasm_bindgen_test]
    fn custom_erc20_maps_inputs_to_owned_core_asset() {
        let address = Ts::new_unchecked(JsValue::from_str(
            "0x00000000000000000000000000000000000000AB",
        ));
        let asset = Asset::erc20(&address, " Tkn ".to_owned(), 6_u8.into()).unwrap();

        let CoreAsset::Erc20 {
            symbol,
            decimals,
            address,
            ..
        } = &asset.inner
        else {
            panic!("expected ERC20 asset");
        };
        assert!(matches!(symbol, Cow::Owned(_)));
        assert_eq!(symbol, " Tkn ");
        assert_eq!(*decimals, 6);
        assert_eq!(*address, Address::with_last_byte(0xab));
    }

    #[wasm_bindgen_test]
    fn custom_erc20_rejects_invalid_address_length() {
        let address = Ts::new_unchecked(JsValue::from_str("0x01"));

        assert!(Asset::erc20(&address, "xyz".to_owned(), 6_u8.into()).is_err());
    }

    #[wasm_bindgen_test]
    fn custom_erc20_reuses_decimals_validation() {
        let address = Ts::new_unchecked(JsValue::from_str(
            "0x00000000000000000000000000000000000000ab",
        ));

        assert!(
            Asset::erc20(
                &address,
                "xyz".to_owned(),
                JsValue::from_f64(6.5).unchecked_into()
            )
            .is_err()
        );
    }

    #[wasm_bindgen_test]
    fn native_assets_have_no_token_address() {
        for asset in [Asset::eth(), Asset::matic()] {
            let kind: JsValue = asset.kind().unwrap().into();
            assert_eq!(kind.as_string().unwrap(), "native");
            let address: JsValue = asset.address().unwrap().into();
            assert!(address.is_undefined());
        }
    }

    #[wasm_bindgen_test]
    fn erc20_asset_exposes_token_address_as_checksummed_hex() {
        let asset = Asset::ethereum_dai();

        let kind: JsValue = asset.kind().unwrap().into();
        assert_eq!(kind.as_string().unwrap(), "erc20");
        let address: JsValue = asset.address().unwrap().into();
        assert_eq!(
            address.as_string().unwrap(),
            "0x6B175474E89094C44Da98b954EedeAC495271d0F"
        );
    }
}
