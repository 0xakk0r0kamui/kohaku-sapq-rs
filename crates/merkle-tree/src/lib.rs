#![doc = include_str!("../README.md")]

use std::array::from_fn;

use crate::{element::Element, hasher::Hasher};

pub mod element;
pub mod hasher;
pub mod proof;

/// Number of parents in a level below which parallel hashing costs more than it saves.
const PARALLEL_THRESHOLD: usize = 256;

/// A Merkle tree with a fixed depth and arity.
#[derive(Clone)]
pub struct MerkleTree<const DEPTH: usize, const ARITY: usize, E: Element, H: Hasher<ARITY, E>> {
    /// `nodes[0]` holds the leaves; `nodes[level]` holds the hashes at `level`.
    nodes: Vec<Vec<E>>,

    /// The hash of an empty subtree at each level, `0..=DEPTH`.
    zeros: Vec<E>,

    phantom: std::marker::PhantomData<H>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MerkleTreeError {
    #[error("Merkle tree is full")]
    TreeFull,
    #[error("Missing leaf element")]
    MissingLeaf,
    #[error("Index {0} is out of bounds")]
    IndexOutOfBounds(usize),
    #[error("Other: {0}")]
    Other(String),
}

impl<const DEPTH: usize, const ARITY: usize, E: Element, H: Hasher<ARITY, E>>
    MerkleTree<DEPTH, ARITY, E, H>
{
    /// Creates a new Merkle tree.
    pub fn new() -> Self {
        const {
            assert!(ARITY >= 2, "ARITY must be at least 2");
            assert!(ARITY <= 256, "ARITY must be at most 256");
        }

        Self {
            nodes: vec![Vec::new(); DEPTH + 1],
            zeros: zero_hashes::<ARITY, E, H>(DEPTH),
            phantom: std::marker::PhantomData,
        }
    }

    /// Creates a new Merkle tree from the given leaves.
    pub fn from_leaves(leaves: &[E]) -> Result<Self, MerkleTreeError> {
        let mut tree = Self::new();
        tree.splice(0, leaves)?;
        Ok(tree)
    }

    /// Returns the tree's root hash.
    pub fn root(&self) -> Result<E, MerkleTreeError> {
        Ok(self.node(DEPTH, 0))
    }

    /// Returns the tree's leaves.
    pub fn leaves(&self) -> &[E] {
        &self.nodes[0]
    }

    /// Returns the tree's nodes.
    pub fn nodes(&self) -> &[Vec<E>] {
        &self.nodes
    }

    /// Returns the inclusion proof for the leaf at the given index.
    ///
    /// # Errors
    /// Returns an error if the index is out of bounds.
    pub fn proof(
        &self,
        index: usize,
    ) -> Result<proof::MerkleProof<DEPTH, ARITY, E>, MerkleTreeError> {
        let len = self.nodes[0].len();
        if index >= len {
            return Err(MerkleTreeError::IndexOutOfBounds(index));
        }

        let mut path = [0u8; DEPTH];
        let mut siblings: [[E; ARITY]; DEPTH] = from_fn(|_| from_fn(|_| E::default()));
        let mut idx = index;

        for level in 0..DEPTH {
            let parent = idx / ARITY;
            path[level] = (idx % ARITY) as u8;
            siblings[level] = from_fn(|i| self.node(level, parent * ARITY + i));
            idx = parent;
        }

        let leaf = self.nodes[0][index].clone();
        Ok(proof::MerkleProof::new(leaf, siblings, path, self.root()?))
    }

    /// Inserts a single leaf at `index`. If a leaf already exists at that index, it is
    /// replaced.
    ///
    /// # Errors
    /// Returns an error if the index is out of bounds or the tree is full.
    pub fn insert(&mut self, index: usize, leaf: E) -> Result<(), MerkleTreeError> {
        self.splice(index, &[leaf])
    }

    /// Splices in `leaves` starting at `index`. If any leaves already exist at those indices,
    /// they are replaced. Spliced leaves must be contiguous.
    ///
    /// # Errors
    /// Returns an error if the index is out of bounds or the tree is full.
    pub fn splice(&mut self, index: usize, leaves: &[E]) -> Result<(), MerkleTreeError> {
        if leaves.is_empty() {
            return Ok(());
        }

        let len = self.nodes[0].len();
        if index > len {
            return Err(MerkleTreeError::IndexOutOfBounds(index));
        }

        let capacity = ARITY.checked_pow(DEPTH as u32).unwrap_or(usize::MAX);
        let new_len = (index + leaves.len()).max(len);
        if new_len > capacity {
            return Err(MerkleTreeError::TreeFull);
        }

        self.nodes[0].resize(new_len, self.zeros[0].clone());
        self.nodes[0][index..index + leaves.len()].clone_from_slice(leaves);

        let mut start = index;
        let mut end = index + leaves.len();

        for level in 1..=DEPTH {
            start /= ARITY;
            end = end.div_ceil(ARITY);

            let level_len = end.max(self.nodes[level].len());
            self.nodes[level].resize(level_len, self.zeros[level].clone());

            let (lower, upper) = self.nodes.split_at_mut(level);
            hash_level::<ARITY, E, H>(
                &lower[level - 1][start * ARITY..],
                &mut upper[0][start..end],
                &self.zeros[level - 1],
                PARALLEL_THRESHOLD,
            );
        }

        Ok(())
    }

    /// Returns the node at (`level`, `index`), falling back to the empty-subtree hash for
    /// indices past the end of that level.
    fn node(&self, level: usize, index: usize) -> E {
        self.nodes[level]
            .get(index)
            .cloned()
            .unwrap_or(self.zeros[level].clone())
    }
}

impl<const DEPTH: usize, const ARITY: usize, E: Element + PartialEq, H: Hasher<ARITY, E>>
    MerkleTree<DEPTH, ARITY, E, H>
{
    /// Returns the inclusion proof for the given leaf.
    ///
    /// # Errors
    /// Returns an error if the leaf is not found in the tree.
    pub fn leaf_proof(
        &self,
        leaf: E,
    ) -> Result<proof::MerkleProof<DEPTH, ARITY, E>, MerkleTreeError> {
        let len = self.nodes[0].len();
        for index in 0..len {
            if self.nodes[0][index] == leaf {
                return self.proof(index);
            }
        }

        Err(MerkleTreeError::MissingLeaf)
    }
}

impl<const DEPTH: usize, const ARITY: usize, E: Element, H: Hasher<ARITY, E>> Default
    for MerkleTree<DEPTH, ARITY, E, H>
{
    fn default() -> Self {
        Self::new()
    }
}

/// Hashes each `ARITY`-wide chunk of `children` into the matching slot of `parents`.
#[cfg_attr(not(feature = "parallel"), expect(unused_variables))]
fn hash_level<const ARITY: usize, E: Element, H: Hasher<ARITY, E>>(
    children: &[E],
    parents: &mut [E],
    zero: &E,
    parallel_threshold: usize,
) {
    #[cfg(feature = "parallel")]
    if parents.len() >= parallel_threshold {
        use rayon::prelude::*;

        parents
            .par_iter_mut()
            .zip(children.par_chunks(ARITY))
            .for_each(|(parent, chunk)| *parent = H::hash(pad_chunk(chunk, zero)));
        return;
    }

    for (parent, chunk) in parents.iter_mut().zip(children.chunks(ARITY)) {
        *parent = H::hash(pad_chunk(chunk, zero));
    }
}

/// Widens a chunk of children to a full `ARITY` array, filling with `zero`.
fn pad_chunk<const ARITY: usize, E: Element>(chunk: &[E], zero: &E) -> [E; ARITY] {
    from_fn(|i| chunk.get(i).cloned().unwrap_or_else(|| zero.clone()))
}

/// Computes the hash of a fully-empty subtree at each height from `0` to `levels` (inclusive).
fn zero_hashes<const ARITY: usize, E: Element, H: Hasher<ARITY, E>>(levels: usize) -> Vec<E> {
    let mut zeros = Vec::with_capacity(levels + 1);
    zeros.push(H::zero());
    for i in 1..=levels {
        let child = &zeros[i - 1];
        let parent = H::hash(from_fn(|_| child.clone()));
        zeros.push(parent);
    }
    zeros
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hasher::Hasher;

    type TestTree = MerkleTree<3, 2, u128, TestHasher>;

    #[derive(Copy, Clone)]
    struct TestHasher;

    impl<const ARITY: usize> Hasher<ARITY, u128> for TestHasher {
        fn hash(children: [u128; ARITY]) -> u128 {
            children
                .into_iter()
                .enumerate()
                .fold(0, |acc, (i, c)| acc ^ c.rotate_left(i as u32))
        }

        fn zero() -> u128 {
            10
        }
    }

    fn tree() -> TestTree {
        TestTree::new()
    }

    #[test]
    fn root_is_deterministic() {
        let tree = tree();
        assert_eq!(tree.root().unwrap(), 102);
    }

    #[test]
    fn from_leaves_reproduces_tree() {
        let leaves = [1, 2, 3];

        let mut tree_a = tree();
        tree_a.splice(0, &leaves).unwrap();

        let tree_b = TestTree::from_leaves(tree_a.leaves()).unwrap();

        assert_eq!(tree_a.root().unwrap(), tree_b.root().unwrap());
    }

    #[test]
    fn insert_changes_root() {
        let mut tree = tree();
        let root_a = tree.root().unwrap();

        tree.insert(0, 1).unwrap();
        let root_b = tree.root().unwrap();

        tree.insert(0, 2).unwrap();
        let root_c = tree.root().unwrap();

        assert_ne!(root_a, root_b);
        assert_ne!(root_b, root_c);
    }

    #[test]
    fn insert_and_splice_produce_same_root() {
        let leaves = [1, 2, 3];

        let mut inserted = tree();
        for (i, &l) in leaves.iter().enumerate() {
            inserted.insert(i, l).unwrap();
        }

        let mut spliced = tree();
        spliced.splice(0, &leaves).unwrap();

        assert_eq!(inserted.root().unwrap(), spliced.root().unwrap());
    }

    #[test]
    fn splice_idempotent() {
        let leaves = [1, 2, 3];
        let mut full = tree();
        full.splice(0, &leaves).unwrap();

        let root_before = full.root().unwrap();
        full.splice(0, &leaves).unwrap();

        assert_eq!(full.root().unwrap(), root_before);
    }

    #[test]
    fn inserting_same_leaf_no_op() {
        let mut tree = tree();
        tree.insert(0, 1).unwrap();
        let root_before = tree.root().unwrap();

        tree.insert(0, 1).unwrap();
        assert_eq!(tree.root().unwrap(), root_before);
    }

    #[test]
    fn inserting_different_leaf_replaces_it() {
        let mut tree = tree();
        tree.insert(0, 1).unwrap();
        let root_before = tree.root().unwrap();

        tree.insert(0, 2).unwrap();

        assert_ne!(tree.root().unwrap(), root_before);
        assert_eq!(tree.leaf_proof(2).unwrap().leaf, 2);
    }

    #[test]
    fn parallel_and_sequential_hash_level_agree() {
        let children: Vec<u128> = (1..=127).collect();
        let zero = 7;

        let mut serial = vec![0; 64];
        hash_level::<2, u128, TestHasher>(&children, &mut serial, &zero, usize::MAX);

        let mut parallel = vec![0; 64];
        hash_level::<2, u128, TestHasher>(&children, &mut parallel, &zero, 0);

        assert_eq!(serial, parallel);
    }

    #[test]
    fn insert_beyond_capacity_errors() {
        let mut tree = MerkleTree::<2, 2, u128, TestHasher>::new();
        tree.splice(0, &[1, 2, 3, 4]).unwrap();

        assert!(matches!(tree.insert(4, 5), Err(MerkleTreeError::TreeFull)));
    }

    #[test]
    fn leaf_proof_missing_leaf_errors() {
        let mut tree = tree();
        tree.insert(0, 1).unwrap();

        assert!(matches!(
            tree.leaf_proof(9),
            Err(MerkleTreeError::MissingLeaf)
        ));
    }

    #[test]
    fn proof_out_of_bounds_errors() {
        let mut tree = tree();
        tree.insert(0, 1).unwrap();

        assert!(matches!(
            tree.proof(5),
            Err(MerkleTreeError::IndexOutOfBounds(5))
        ));
    }
}
