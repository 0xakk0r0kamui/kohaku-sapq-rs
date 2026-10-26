use kohaku_tornadocash::Deposit as CoreDeposit;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::{call::Call, note::Note, pool::Pool};

/// A Tornado Cash deposit.
///
/// See [`kohaku_tornadocash::Deposit`] for more details.
#[wasm_bindgen]
pub struct Deposit {
    inner: CoreDeposit,
}

#[wasm_bindgen]
impl Deposit {
    /// Construct an independently owned deposit without consuming the pool or note.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(pool: &Pool, note: &Note) -> Self {
        Self {
            inner: CoreDeposit::new(&pool.inner, note.inner.clone()),
        }
    }

    /// Return the core's deposit call as plain JavaScript data.
    #[wasm_bindgen(js_name = depositData)]
    pub fn call(&self) -> Result<Ts<Call>, JsError> {
        Ok(Call::from(self.inner.call()).into_ts()?)
    }

    /// Return the core's ERC20 approval call, or undefined for native pools.
    ///
    /// This prepares call data without checking existing allowance.
    #[wasm_bindgen(js_name = approvalData)]
    pub fn approval(&self) -> Result<Option<Ts<Call>>, JsError> {
        Ok(self
            .inner
            .approval()
            .map(|call| Call::from(call).into_ts())
            .transpose()?)
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use js_sys::Reflect;
    use kohaku_tornadocash::{Call as CoreCall, Note as CoreNote, Pool as CorePool};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test]
    fn deposit_data_maps_native_and_erc20_core_calls() {
        for pool in [CorePool::ETHEREUM_ETHER_01, CorePool::ETHEREUM_DAI_100] {
            let (deposit, core) = deposit_fixture(pool);

            assert_call(deposit.call().unwrap(), core.call());
        }
    }

    #[wasm_bindgen_test]
    fn native_approval_data_is_undefined() {
        let (deposit, _) = deposit_fixture(CorePool::ETHEREUM_ETHER_01);
        let approval: JsValue = deposit.approval().unwrap().into();

        assert!(approval.is_undefined());
    }

    #[wasm_bindgen_test]
    fn erc20_approval_data_maps_core_call() {
        let (deposit, core) = deposit_fixture(CorePool::ETHEREUM_DAI_100);

        assert_call(
            deposit.approval().unwrap().unwrap(),
            core.approval().unwrap(),
        );
    }

    fn assert_call(actual: Ts<Call>, expected: CoreCall) {
        let actual: JsValue = actual.into();
        let expected: JsValue = Call::from(expected).into_ts().unwrap().into();
        for field in ["to", "data", "value"] {
            assert_eq!(
                Reflect::get(&actual, &field.into()).unwrap(),
                Reflect::get(&expected, &field.into()).unwrap()
            );
        }
    }

    fn deposit_fixture(core_pool: CorePool) -> (Deposit, CoreDeposit) {
        let core_note = CoreNote::new([1_u8; 31], [2_u8; 31]);
        let core = CoreDeposit::new(&core_pool, core_note.clone());
        let pool = Pool { inner: core_pool };
        let note = Note { inner: core_note };

        (Deposit::new(&pool, &note), core)
    }
}
