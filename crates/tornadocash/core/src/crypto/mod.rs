use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use num_bigint::{BigInt, Sign};
use ruint::aliases::U256;

mod babyjubjub;
pub mod mimc;
mod mimc_constants;
pub mod pedersen;

fn test_bit(bytes: &[u8], i: usize) -> bool {
    bytes[i / 8] & (1 << (i % 8)) != 0
}

fn u256_to_num_bigint(x: U256) -> BigInt {
    BigInt::from_bytes_le(Sign::Plus, &x.to_le_bytes::<32>())
}

fn fr_to_num_bigint(f: Fr) -> BigInt {
    let le = f.into_bigint().to_bytes_le();
    BigInt::from_bytes_le(Sign::Plus, &le)
}
