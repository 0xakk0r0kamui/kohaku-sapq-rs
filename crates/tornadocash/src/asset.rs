use std::borrow::Cow;

use alloy::primitives::{Address, address};
use serde::{Deserialize, Serialize};

/// Represents an asset in a tornadocash pool.
///
/// Assets can either be native (e.g. ETH, MATIC) or ERC20 tokens.
///
/// Defines a set of known Tornado Cash assets as constants. The constants are
/// named in the format `<ASSET>` for native tokens, and `<CHAIN>_<ASSET>` for ERC20
/// tokens. E.g. `ETH` for native ETH, and `ETHEREUM_DAI` for the DAI token on
/// Ethereum mainnet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Asset {
    Native {
        symbol: Cow<'static, str>,
        decimals: u8,
    },
    Erc20 {
        address: Address,
        symbol: Cow<'static, str>,
        decimals: u8,
    },
}

impl Asset {
    pub const ETH: Asset = Asset::Native {
        symbol: Cow::Borrowed("eth"),
        decimals: 18,
    };

    pub const MATIC: Asset = Asset::Native {
        symbol: Cow::Borrowed("matic"),
        decimals: 18,
    };

    pub const ETHEREUM_DAI: Asset = Asset::Erc20 {
        address: address!("0x6B175474E89094C44Da98b954EedeAC495271d0F"),
        symbol: Cow::Borrowed("dai"),
        decimals: 18,
    };

    pub const ETHEREUM_USDC: Asset = Asset::Erc20 {
        address: address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        symbol: Cow::Borrowed("usdc"),
        decimals: 6,
    };

    pub const ETHEREUM_USDT: Asset = Asset::Erc20 {
        address: address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
        symbol: Cow::Borrowed("usdt"),
        decimals: 6,
    };

    pub const ETHEREUM_WBTC: Asset = Asset::Erc20 {
        address: address!("0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"),
        symbol: Cow::Borrowed("wbtc"),
        decimals: 8,
    };

    pub const SEPOLIA_DAI: Asset = Asset::Erc20 {
        address: address!("0xFF34B3d4Aee8ddCd6F9AFFFB6Fe49bD371b8a357"),
        symbol: Cow::Borrowed("dai"),
        decimals: 18,
    };

    #[must_use]
    pub const fn address(&self) -> Option<Address> {
        match self {
            Asset::Native { .. } => None,
            Asset::Erc20 { address, .. } => Some(*address),
        }
    }

    #[must_use]
    pub const fn symbol(&self) -> &str {
        match self {
            Asset::Native { symbol, .. } | Asset::Erc20 { symbol, .. } => match symbol {
                Cow::Borrowed(s) => s,
                Cow::Owned(s) => s.as_str(),
            },
        }
    }

    #[must_use]
    pub const fn decimals(&self) -> u8 {
        match self {
            Asset::Native { decimals, .. } | Asset::Erc20 { decimals, .. } => *decimals,
        }
    }
}
