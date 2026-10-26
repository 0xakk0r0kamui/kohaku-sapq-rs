use kohaku_tornadocash::{Note as CoreNote, Nullifier, Secret};
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::hex::Hex;

/// A tornadocash deposit note.
///
/// See [`kohaku_tornadocash::Note`] for more details.
#[wasm_bindgen]
pub struct Note {
    pub(crate) inner: CoreNote,
}

#[wasm_bindgen]
impl Note {
    /// Construct a note from two 31-byte, 0x-prefixed hex strings.
    ///
    /// # Errors
    ///
    /// Throws if either input cannot be decoded into a 31-byte secret.
    #[wasm_bindgen(constructor)]
    pub fn new(nullifier: &Ts<Hex>, secret: &Ts<Hex>) -> Result<Note, JsError> {
        let nullifier_hex = nullifier.to_rust()?;
        let secret_hex = secret.to_rust()?;
        let nullifier = Nullifier::try_from(nullifier_hex)?;
        let secret = Secret::try_from(secret_hex)?;

        Ok(Self {
            inner: CoreNote::new(nullifier, secret),
        })
    }
    #[wasm_bindgen(getter)]
    pub fn nullifier(&self) -> Result<Ts<Hex>, JsError> {
        let hex = Hex::from(self.inner.nullifier);

        Ok(hex.into_ts()?)
    }

    #[wasm_bindgen(getter)]
    pub fn secret(&self) -> Result<Ts<Hex>, JsError> {
        let hex = Hex::from(self.inner.secret);

        Ok(hex.into_ts()?)
    }

    /// Return the 62-byte preimage encoded as hex
    pub fn preimage(&self) -> Result<Ts<Hex>, JsError> {
        let bytes = self.inner.preimage();
        let hex = Hex::from(bytes.as_slice());

        Ok(hex.into_ts()?)
    }

    /// Compute the commitment encoded as hex
    pub fn commitment(&self) -> Result<Ts<Hex>, JsError> {
        let hex = Hex::from(self.inner.commitment());

        Ok(hex.into_ts()?)
    }

    /// Compute the nullifier hash encoded as hex
    ///
    /// This does not check whether the note has been spent.
    #[wasm_bindgen(js_name = nullifierHash)]
    pub fn nullifier_hash(&self) -> Result<Ts<Hex>, JsError> {
        let hex = Hex::from(self.inner.nullifier_hash());

        Ok(hex.into_ts()?)
    }
}
