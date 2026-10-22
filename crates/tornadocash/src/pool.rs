use std::fmt::Display;

use alloy::primitives::{Address, address};
use serde::{Deserialize, Serialize};

use crate::{asset::Asset, note::NoteString};

/// Represents a tornadocash pool.
///
/// Pools are uniquely defined by their `chain_id`, `asset` symbol, and `amount`.
///
/// Defines a set of known Tornado Cash pools as constants. The constants are
/// named in the format `<CHAIN>_<ASSET>_<AMOUNT>`, e.g. `ETHEREUM_ETHER_01` for
/// the 0.1 ETH pool on Ethereum mainnet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Pool {
    pub chain_id: u64,
    pub address: Address,
    pub asset: Asset,
    pub amount_wei: u128,
    pub deployed_block: u64,

    pub paymaster: Option<PaymasterInfo>,
}

/// Privacy-paymaster information for a pool.
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PaymasterInfo {
    pub address: Address,
    pub adapter: Address,
}

#[allow(clippy::unreadable_literal)]
impl Pool {
    /// List of default Tornado Cash pools.
    pub const POOLS: &[Pool] = &[
        Pool::SEPOLIA_ETHER_01,
        Pool::SEPOLIA_ETHER_1,
        Pool::SEPOLIA_ETHER_10,
        Pool::SEPOLIA_DAI_100,
        Pool::ETHEREUM_ETHER_01,
        Pool::ETHEREUM_ETHER_1,
        Pool::ETHEREUM_ETHER_10,
        Pool::ETHEREUM_ETHER_100,
        Pool::POLYGON_MATIC_100,
        Pool::POLYGON_MATIC_1000,
        Pool::ETHEREUM_DAI_100,
        Pool::ETHEREUM_DAI_1000,
        Pool::ETHEREUM_DAI_10000,
        Pool::ETHEREUM_DAI_100000,
        Pool::ETHEREUM_USDC_100,
        Pool::ETHEREUM_USDC_1000,
        Pool::ETHEREUM_USDT_100,
        Pool::ETHEREUM_USDT_1000,
        Pool::ETHEREUM_WBTC_01,
        Pool::ETHEREUM_WBTC_1,
        Pool::ETHEREUM_WBTC_10,
    ];

    pub const SEPOLIA_ETHER_01: Pool = Pool {
        chain_id: 11155111,
        address: address!("0x8C4A04d872a6C1BE37964A21ba3a138525dFF50b"),
        asset: Asset::ETH,
        amount_wei: 10_u128.pow(17),
        deployed_block: 5_594_400,
        paymaster: Some(PaymasterInfo {
            address: address!("0x1c5aCCb9c09D72945b79EC986776136bE01d7B2F"),
            adapter: address!("0xa616aAE443FCCABfc2F1EA2Afe001E5046FFDCe0"),
        }),
    };

    pub const SEPOLIA_ETHER_1: Pool = Pool {
        chain_id: 11155111,
        address: address!("0x8cc930096B4Df705A007c4A039BDFA1320Ed2508"),
        asset: Asset::ETH,
        amount_wei: 10_u128.pow(18),
        deployed_block: 5_594_401,
        paymaster: Some(PaymasterInfo {
            address: address!("0x1c5aCCb9c09D72945b79EC986776136bE01d7B2F"),
            adapter: address!("0x67a898343F32641206d0f30CB3367944a8919A3A"),
        }),
    };

    pub const SEPOLIA_ETHER_10: Pool = Pool {
        chain_id: 11155111,
        address: address!("0x8D10d506D29Fc62ABb8A290B99F66dB27Fc43585"),
        asset: Asset::ETH,
        amount_wei: 10_u128.pow(19),
        deployed_block: 5_594_402,
        paymaster: None,
    };

    pub const SEPOLIA_DAI_100: Pool = Pool {
        chain_id: 11155111,
        address: address!("0x6921fd1a97441dd603a997ED6DDF388658daf754"),
        asset: Asset::SEPOLIA_DAI,
        amount_wei: 10_u128.pow(20),
        deployed_block: 5_594_406,
        paymaster: Some(PaymasterInfo {
            address: address!("0x1c5aCCb9c09D72945b79EC986776136bE01d7B2F"),
            adapter: address!("0xbF0a7969dacF8337716d0F283df0574dF56b0479"),
        }),
    };

    pub const ETHEREUM_ETHER_01: Pool = Pool {
        chain_id: 1,
        address: address!("0x12D66f87A04A9E220743712cE6d9bB1B5616B8Fc"),
        asset: Asset::ETH,
        amount_wei: 10_u128.pow(17),
        deployed_block: 9116966,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0x4a3F73a23563bB467Aa034d0fAB61e7BcDf26161"),
        }),
    };

    pub const ETHEREUM_ETHER_1: Pool = Pool {
        chain_id: 1,
        address: address!("0x47CE0C6eD5B0Ce3d3A51fdb1C52DC66a7c3c2936"),
        asset: Asset::ETH,
        amount_wei: 10_u128.pow(18),
        deployed_block: 9_117_609,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0xC086b50Dd859Bb15D553327178DEdbC347B5b905"),
        }),
    };

    pub const ETHEREUM_ETHER_10: Pool = Pool {
        chain_id: 1,
        address: address!("0x910Cbd523D972eb0a6f4cAe4618aD62622b39DbF"),
        asset: Asset::ETH,
        amount_wei: 10_u128.pow(19),
        deployed_block: 9_117_720,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0x08d1103bff53CBdC9DB1e81B79475CEad2e35870"),
        }),
    };

    pub const ETHEREUM_ETHER_100: Pool = Pool {
        chain_id: 1,
        address: address!("0xA160cdAB225685dA1d56aa342Ad8841c3b53f291"),
        asset: Asset::ETH,
        amount_wei: 10_u128.pow(20),
        deployed_block: 9_161_895,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0x3aF851f9BeEeE6E9fa06022f34Dc4C453D265aE5"),
        }),
    };

    pub const ETHEREUM_DAI_100: Pool = Pool {
        chain_id: 1,
        address: address!("0xD4B88Df4D29F5CedD6857912842cff3b20C8Cfa3"),
        asset: Asset::ETHEREUM_DAI,
        amount_wei: 10_u128.pow(20),
        deployed_block: 9_117_612,
        paymaster: None,
    };

    pub const ETHEREUM_DAI_1000: Pool = Pool {
        chain_id: 1,
        address: address!("0xFD8610d20aA15b7B2E3Be39B396a1bC3516c7144"),
        asset: Asset::ETHEREUM_DAI,
        amount_wei: 10_u128.pow(21),
        deployed_block: 9_161_917,
        paymaster: None,
    };

    pub const ETHEREUM_DAI_10000: Pool = Pool {
        chain_id: 1,
        address: address!("0x07687e702b410Fa43f4cB4Af7FA097918ffD2730"),
        asset: Asset::ETHEREUM_DAI,
        amount_wei: 10_u128.pow(22),
        deployed_block: 12_066_007,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0x36A5b4bCE77700B9C52cFEE53Fc8cA3B58eb7E35"),
        }),
    };

    pub const ETHEREUM_DAI_100000: Pool = Pool {
        chain_id: 1,
        address: address!("0x23773E65ed146A459791799d01336DB287f25334"),
        asset: Asset::ETHEREUM_DAI,
        amount_wei: 10_u128.pow(23),
        deployed_block: 12_066_048,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0x83Ff713F0939409E1c6023bc8aaB3E49eD968c8D"),
        }),
    };

    pub const ETHEREUM_USDC_100: Pool = Pool {
        chain_id: 1,
        address: address!("0xd96f2B1c14Db8458374d9Aca76E26c3D18364307"),
        asset: Asset::ETHEREUM_USDC,
        amount_wei: 10_u128.pow(8),
        deployed_block: 9_161_958,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0x44336242f9766b4c751F6958499C5cc18325e25B"),
        }),
    };

    pub const ETHEREUM_USDC_1000: Pool = Pool {
        chain_id: 1,
        address: address!("0x4736dCf1b7A3d580672CcE6E7c65cd5cc9cFBa9D"),
        asset: Asset::ETHEREUM_USDC,
        amount_wei: 10_u128.pow(9),
        deployed_block: 9_161_965,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0xB1C7e712AF153073979AdC5ef8B1Cc5A7b0cf031"),
        }),
    };

    pub const ETHEREUM_USDT_100: Pool = Pool {
        chain_id: 1,
        address: address!("0x169AD27A470D064DEDE56a2D3ff727986b15D52B"),
        asset: Asset::ETHEREUM_USDT,
        amount_wei: 10_u128.pow(8),
        deployed_block: 9_162_005,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0xF006862cCa4829D3f3ddD52d93fC49C03D380a5F"),
        }),
    };

    pub const ETHEREUM_USDT_1000: Pool = Pool {
        chain_id: 1,
        address: address!("0x0836222F2B2B24A3F36f98668Ed8F0B38D1a872f"),
        asset: Asset::ETHEREUM_USDT,
        amount_wei: 10_u128.pow(9),
        deployed_block: 9_162_012,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0x5DF524b6c9303eD94A7767c2eD787ee223c9bC54"),
        }),
    };

    pub const ETHEREUM_WBTC_01: Pool = Pool {
        chain_id: 1,
        address: address!("0x178169B423a011fff22B9e3F3abeA13414dDD0F1"),
        asset: Asset::ETHEREUM_WBTC,
        amount_wei: 10_u128.pow(7),
        deployed_block: 12_067_529,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0x0fdF2f686B600523F9752aEca0abb6AD9603A22C"),
        }),
    };

    pub const ETHEREUM_WBTC_1: Pool = Pool {
        chain_id: 1,
        address: address!("0x610B717796ad172B316836AC95a2ffad065CeaB4"),
        asset: Asset::ETHEREUM_WBTC,
        amount_wei: 10_u128.pow(8),
        deployed_block: 12_066_652,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0xec3Cca694EEe55808B3f5EFEDd0bC03E52e47834"),
        }),
    };

    pub const ETHEREUM_WBTC_10: Pool = Pool {
        chain_id: 1,
        address: address!("0xbB93e510BbCD0B7beb5A853875f9eC60275CF498"),
        asset: Asset::ETHEREUM_WBTC,
        amount_wei: 10_u128.pow(9),
        deployed_block: 12_067_591,
        paymaster: Some(PaymasterInfo {
            address: address!("0xe06CB96C57D2442f8F60F5017354BC08F7e91308"),
            adapter: address!("0xC2C427E6ADF66F412921C17958CeDD9C7710F521"),
        }),
    };

    pub const POLYGON_MATIC_100: Pool = Pool {
        chain_id: 137,
        address: address!("0x1E34A77868E19A6647b1f2F47B51ed72dEDE95DD"),
        asset: Asset::MATIC,
        amount_wei: 10_u128.pow(20),
        deployed_block: 16_258_013,
        paymaster: None,
    };

    pub const POLYGON_MATIC_1000: Pool = Pool {
        chain_id: 137,
        address: address!("0xdf231d99Ff8b6c6CBF4E9B9a945CBAcEF9339178"),
        asset: Asset::MATIC,
        amount_wei: 10_u128.pow(21),
        deployed_block: 16_258_032,
        paymaster: None,
    };

    /// Creates a pool from the note's pool hints.
    #[must_use]
    pub fn from_note(note: &NoteString) -> Option<Self> {
        Pool::POOLS
            .iter()
            .find(|pool| {
                pool.chain_id == note.chain_id
                    && pool.symbol() == note.symbol
                    && pool.amount() == note.amount
            })
            .cloned()
    }

    /// Creates a pool from the provided address.
    #[must_use]
    pub fn from_address(address: Address) -> Option<Self> {
        Pool::POOLS
            .iter()
            .find(|pool| pool.address == address)
            .cloned()
    }

    /// Pool ID, e.g. "eth-0.1-1" for the 0.1 ETH pool on Ethereum mainnet.
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}-{}-{}", self.symbol(), self.amount(), self.chain_id)
    }

    /// Pool asset symbol, e.g. "eth" or "matic"
    #[must_use]
    pub fn symbol(&self) -> &str {
        self.asset.symbol()
    }

    /// Decimal amount as a string, e.g. "0.1"
    #[must_use]
    pub fn amount(&self) -> String {
        format_amount(self.amount_wei, self.asset.decimals())
    }
}

impl Display for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.id())
    }
}

fn format_amount(amount: u128, decimals: u8) -> String {
    if decimals == 0 {
        return amount.to_string();
    }

    let divisor = 10u128.pow(u32::from(decimals));

    let whole = amount / divisor;
    let frac = amount % divisor;

    if frac == 0 {
        return whole.to_string();
    }

    // Pad fractional part with leading zeros
    let decimals = decimals as usize;
    let mut frac_str = format!("{frac:0decimals$}");

    // Trim trailing zeros
    while frac_str.ends_with('0') {
        frac_str.pop();
    }

    format!("{whole}.{frac_str}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_id() {
        let pool = Pool::ETHEREUM_ETHER_01;
        assert_eq!(pool.id(), "eth-0.1-1");
    }
}
