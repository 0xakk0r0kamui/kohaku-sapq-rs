use std::sync::LazyLock;

use websnark_rs::{circuit::Circuit, proving_key::ProvingKey};

const CIRCUIT_BR: &[u8] = include_bytes!("../data/circuit.bin.br");
const PROVING_KEY_BR: &[u8] = include_bytes!("../data/proving_key.bin.br");

static CIRCUIT: LazyLock<Circuit> = LazyLock::new(|| {
    let data = decompress(CIRCUIT_BR);
    postcard::from_bytes(&data).expect("embedded circuit is valid")
});

static PROVING_KEY: LazyLock<ProvingKey> = LazyLock::new(|| {
    let data = decompress(PROVING_KEY_BR);
    postcard::from_bytes(&data).expect("embedded proving key is valid")
});

/// Returns the embedded tornadocash-classic circuit.
#[must_use]
pub fn circuit() -> &'static Circuit {
    &CIRCUIT
}

/// Returns the embedded tornadocash-classic proving key.
#[must_use]
pub fn proving_key() -> &'static ProvingKey {
    &PROVING_KEY
}

fn decompress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    brotli::BrotliDecompress(&mut &data[..], &mut out).expect("embedded data is valid brotli");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "run with `cargo test --release -- --ignored`"]
    fn loads_embedded_artifacts() {
        let _ = circuit();
        let _ = proving_key();
    }
}
