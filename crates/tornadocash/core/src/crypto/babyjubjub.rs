use ark_bn254::Fr;
use ark_ff::{AdditiveGroup, Field};
use num_bigint::BigInt as NumBigInt;

use crate::crypto::test_bit;

pub(crate) const A: u64 = 168_700;
pub(crate) const D: u64 = 168_696;

#[derive(Clone, Debug)]
pub struct Point {
    pub x: Fr,
    pub y: Fr,
}

#[derive(Clone, Debug)]
pub struct PointProjective {
    pub x: Fr,
    pub y: Fr,
    pub z: Fr,
}

impl Point {
    pub fn projective(&self) -> PointProjective {
        PointProjective {
            x: self.x,
            y: self.y,
            z: Fr::ONE,
        }
    }

    pub fn mul_scalar(&self, n: &NumBigInt) -> Point {
        // double-and-add (same as reference)

        let mut r = PointProjective {
            x: Fr::ZERO,
            y: Fr::ONE,
            z: Fr::ONE,
        };

        let mut exp = self.projective();

        let (_, bytes) = n.to_bytes_le();

        let bits = n.bits() as usize;

        for i in 0..bits {
            if test_bit(&bytes, i) {
                r = r.add(&exp);
            }
            exp = exp.add(&exp);
        }

        r.affine()
    }
}

impl PointProjective {
    pub fn affine(&self) -> Point {
        if self.z == Fr::ZERO {
            return Point {
                x: Fr::ZERO,
                y: Fr::ZERO,
            };
        }

        let zinv = self.z.inverse().unwrap();
        Point {
            x: self.x * zinv,
            y: self.y * zinv,
        }
    }

    #[allow(clippy::many_single_char_names)]
    pub fn add(&self, q: &PointProjective) -> PointProjective {
        // add-2008-bbjlp
        // https://hyperelliptic.org/EFD/g1p/auto-twisted-projective.html#addition-add-2008-bbjlp

        let d = Fr::from(D);
        let a_coeff = Fr::from(A);

        let a = self.z * q.z;
        let b = a.square();
        let c = self.x * q.x;
        let dxy = self.y * q.y;

        let e = d * c * dxy;

        let f = b - e;
        let g = b + e;

        let aux = (self.x + self.y) * (q.x + q.y) - c - dxy;
        let x3 = a * f * aux;

        let ac = a_coeff * c;
        let dac = dxy - ac;
        let y3 = a * g * dac;

        let z3 = f * g;

        PointProjective {
            x: x3,
            y: y3,
            z: z3,
        }
    }
}
