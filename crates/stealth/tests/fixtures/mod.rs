use alloy::primitives::{Address, Bytes};
use kohaku_stealth::{Account, Announcement, SchemeId3, StealthScheme};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Payment {
    pub scheme_id: u64,
    pub meta_address: Bytes,
    pub stealth_address: Address,
    pub spend_key: Bytes,
    pub ephemeral_pub_key: Bytes,
    pub metadata: Bytes,
}

impl Payment {
    pub fn load() -> Self {
        serde_json::from_str(include_str!("payment.json")).unwrap()
    }

    pub fn account(&self) -> Account {
        // The seed used by upstream's emit_payment_json example.
        let seed: [u8; 128] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3));
        let account = Account::from_seed(&seed).unwrap();
        assert_eq!(
            SchemeId3::meta_to_bytes(account.meta_address()),
            self.meta_address.as_ref()
        );
        account
    }

    pub fn announcement(&self) -> Announcement {
        SchemeId3::announcement_from_bytes(
            &self.stealth_address.into_array(),
            &self.ephemeral_pub_key,
            &self.metadata,
        )
        .unwrap()
    }
}
