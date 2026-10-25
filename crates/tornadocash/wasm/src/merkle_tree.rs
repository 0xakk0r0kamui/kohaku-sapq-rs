use kohaku_tornadocash::{Field, merkle_tree::MerkleTree as CoreMerkleTree};
use serde::{Deserialize, Serialize};
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::{bigint::JsU32, hex::Hex, merkle_proof::MerkleProof};

/// An ordered JavaScript array of commitment hex strings, with no WASM ownership.
#[derive(Serialize, Deserialize, Tsify)]
#[serde(transparent)]
pub struct Commitments(Vec<Hex>);

/// A canonical depth-20 binary Tornado tree stored in WASM memory until freed.
///
/// See [`kohaku_tornadocash::merkle_tree::MerkleTree`] for the canonical tree type.
#[wasm_bindgen]
#[derive(Default)]
pub struct MerkleTree {
    inner: CoreMerkleTree,
}

#[wasm_bindgen]
impl MerkleTree {
    /// Construct an independently owned empty Tornado tree.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: CoreMerkleTree::new(),
        }
    }

    /// Construct a tree from commitments in their supplied leaf order.
    ///
    /// # Errors
    ///
    /// Throws if any commitment is not a valid BN254 field element,
    /// or if the core rejects the leaf count.
    #[wasm_bindgen(js_name = fromLeaves)]
    pub fn from_leaves(commitments: &Ts<Commitments>) -> Result<Self, JsError> {
        let leaves = decode_commitments(commitments)?;

        Ok(Self {
            inner: CoreMerkleTree::from_leaves(&leaves)?,
        })
    }

    /// Replace existing leaves or append commitments contiguously from startIndex.
    ///
    /// An empty batch leaves the tree unchanged, including at an index beyond its leaves.
    /// The complete batch is validated before any mutation.
    ///
    /// # Errors
    ///
    /// Throws if startIndex is not a finite integer JS number in the u32 range,
    /// a commitment is invalid, or the core rejects the bounds or capacity.
    pub fn splice(
        &mut self,
        #[wasm_bindgen(js_name = startIndex)] start_index: JsU32,
        commitments: &Ts<Commitments>,
    ) -> Result<(), JsError> {
        let start_index = u32::try_from(start_index)? as usize;
        let leaves = decode_commitments(commitments)?;
        self.inner.splice(start_index, &leaves)?;
        Ok(())
    }

    /// Return the tree's root as lowercase, 32-byte hex.
    pub fn root(&self) -> Result<Ts<Hex>, JsError> {
        Ok(Hex::from(self.inner.root()?).into_ts()?)
    }

    /// Return an independently owned inclusion proof capturing the current root.
    ///
    /// # Errors
    ///
    /// Throws if index is not a finite integer JS number in the u32 range,
    /// or if the core rejects the leaf index.
    pub fn proof(&self, index: JsU32) -> Result<MerkleProof, JsError> {
        let index = u32::try_from(index)? as usize;
        Ok(self.inner.proof(index)?.into())
    }

    /// Return an independently owned inclusion proof for a commitment.
    ///
    /// # Errors
    ///
    /// Throws if the commitment is invalid or is not found in the tree.
    #[wasm_bindgen(js_name = leafProof)]
    pub fn leaf_proof(&self, commitment: &Ts<Hex>) -> Result<MerkleProof, JsError> {
        let leaf = Field::try_from(commitment.to_rust()?)?;
        Ok(self.inner.leaf_proof(leaf)?.into())
    }
}

fn decode_commitments(commitments: &Ts<Commitments>) -> Result<Vec<Field>, JsError> {
    Ok(commitments
        .to_rust()?
        .0
        .into_iter()
        .map(Field::try_from)
        .collect::<Result<Vec<_>, _>>()?)
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use tsify::Tsify;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{Commitments, Field, Hex, MerkleTree};

    #[wasm_bindgen_test]
    fn from_leaves_converts_commitments_in_order() {
        let leaves = [Field::from(0xab), Field::from(1)];
        let commitments = Commitments(leaves.into_iter().map(Hex::from).collect())
            .into_ts()
            .unwrap();

        let tree = MerkleTree::from_leaves(&commitments).unwrap();

        assert_eq!(tree.inner.leaves(), leaves);
    }

    #[wasm_bindgen_test]
    fn splice_converts_the_whole_batch_before_mutating() {
        let initial = Commitments(vec![Hex::from(Field::from(1))])
            .into_ts()
            .unwrap();
        let mut tree = MerkleTree::from_leaves(&initial).unwrap();
        let before = tree.inner.root().unwrap();
        let malformed = Commitments(vec![
            Hex::from(Field::from(2)),
            Hex::from([0xff; 32].as_slice()),
        ])
        .into_ts()
        .unwrap();

        assert!(tree.splice(0_u32.into(), &malformed).is_err());
        assert_eq!(tree.inner.root().unwrap(), before);
        assert_eq!(tree.inner.leaves(), &[Field::from(1)]);
    }

    #[wasm_bindgen_test]
    fn splice_maps_commitments_and_start_index() {
        let initial = Commitments(vec![Hex::from(Field::from(1)), Hex::from(Field::from(2))])
            .into_ts()
            .unwrap();
        let mut tree = MerkleTree::from_leaves(&initial).unwrap();
        let leaves = [Field::from(3), Field::from(4)];
        let batch = Commitments(leaves.into_iter().map(Hex::from).collect())
            .into_ts()
            .unwrap();

        tree.splice(1_u32.into(), &batch).unwrap();

        assert_eq!(tree.inner.leaves(), &[Field::from(1), leaves[0], leaves[1]]);
    }

    #[wasm_bindgen_test]
    fn proof_delegates_the_selected_leaf_and_encodes_metadata() {
        let leaves = [Field::from(1), Field::from(0xab)];
        let batch = Commitments(leaves.into_iter().map(Hex::from).collect())
            .into_ts()
            .unwrap();
        let tree = MerkleTree::from_leaves(&batch).unwrap();

        let proof = tree.proof(1_u32.into()).unwrap();

        let leaf: JsValue = proof.leaf().unwrap().into();
        let root: JsValue = proof.root().unwrap().into();
        let tree_root: JsValue = tree.root().unwrap().into();
        assert_eq!(leaf.as_string().unwrap(), leaves[1].to_string());
        assert_eq!(root, tree_root);

        let commitment = Hex::from(leaves[0]).into_ts().unwrap();
        let leaf_proof = tree.leaf_proof(&commitment).unwrap();
        let leaf: JsValue = leaf_proof.leaf().unwrap().into();
        assert_eq!(leaf.as_string().unwrap(), leaves[0].to_string());
    }
}
