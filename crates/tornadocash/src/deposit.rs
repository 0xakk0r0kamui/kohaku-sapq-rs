use alloy::{rpc::types::TransactionRequest, sol_types::SolCall};
use ruint::aliases::U256;

use crate::{
    Call,
    abis::{erc20::ERC20, tornado::Tornado},
    asset::Asset,
    note::Note,
    pool::Pool,
};

/// A Tornado Cash deposit.
#[derive(Debug, Clone)]
pub struct Deposit {
    pub pool: Pool,
    pub note: Note,
}

impl Deposit {
    #[must_use]
    pub fn new(pool: &Pool, note: Note) -> Self {
        Self {
            pool: pool.clone(),
            note,
        }
    }

    /// Returns the call to deposit this note into the pool.
    ///
    /// For ERC20 pools, the native value is zero since the token is transferred via
    /// a `transferFrom` call.
    #[must_use]
    pub fn deposit_data(&self) -> Call {
        let value = match self.pool.asset {
            Asset::Native { .. } => U256::from(self.pool.amount_wei),
            Asset::Erc20 { .. } => U256::ZERO,
        };
        let deposit_call = Tornado::depositCall {
            _commitment: self.note.commitment().into(),
        };

        Call {
            target: self.pool.address,
            value,
            input: deposit_call.abi_encode().into(),
        }
    }

    /// Returns the ERC20 approval call this deposit requires, if any.
    ///
    /// Native-asset pools return [`None`]. ERC20 pools pull the deposit with `transferFrom`, so
    /// the depositor must approve the pool to spend the pool's denomination first, or the
    /// deposit transaction will revert.
    #[must_use]
    pub fn approval_data(&self) -> Option<Call> {
        let Asset::Erc20 { address, .. } = self.pool.asset else {
            return None;
        };

        let approve_call = ERC20::approveCall {
            spender: self.pool.address,
            amount: U256::from(self.pool.amount_wei),
        };

        Some(Call {
            target: address,
            value: U256::ZERO,
            input: approve_call.abi_encode().into(),
        })
    }
}

impl From<Deposit> for TransactionRequest {
    fn from(deposit: Deposit) -> Self {
        deposit.deposit_data().into()
    }
}

#[cfg(test)]
mod tests {
    use alloy::primitives::B256;

    use super::*;

    #[test]
    fn native_deposit_call_contains_denomination_and_commitment() {
        let deposit = Deposit::new(&Pool::ETHEREUM_ETHER_01, Note::new([1; 31], [2; 31]));
        let call = deposit.deposit_data();

        assert_eq!(call.target, deposit.pool.address);
        assert_eq!(call.value, U256::from(deposit.pool.amount_wei));
        let input = Tornado::depositCall::abi_decode(&call.input).unwrap();
        assert_eq!(input._commitment, B256::from(deposit.note.commitment()));
        assert!(deposit.approval_data().is_none());
    }

    #[test]
    fn erc20_calls_target_pool_and_token_with_zero_native_value() {
        let deposit = Deposit::new(&Pool::ETHEREUM_DAI_100, Note::new([1; 31], [2; 31]));
        let call = deposit.deposit_data();
        let approval = deposit.approval_data().unwrap();
        let Asset::Erc20 { address, .. } = deposit.pool.asset else {
            panic!("expected an ERC20 asset");
        };

        assert_eq!(call.target, deposit.pool.address);
        assert_eq!(call.value, U256::ZERO);
        assert_eq!(approval.target, address);
        assert_eq!(approval.value, U256::ZERO);
        let input = ERC20::approveCall::abi_decode(&approval.input).unwrap();
        assert_eq!(input.spender, deposit.pool.address);
        assert_eq!(input.amount, U256::from(deposit.pool.amount_wei));
    }
}
