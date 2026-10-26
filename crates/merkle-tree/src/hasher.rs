/// A merkle tree hash function.
///
/// The hash function should be collision-resistant and deterministic. It's
/// used to hash a node's children into a parent node, and to provide the zero
/// value for empty leaves.
pub trait Hasher<const ARITY: usize, E> {
    /// Hashes a node's children into its parent hash.
    fn hash(children: [E; ARITY]) -> E;

    /// Returns the zero value for the hash function, which is used as a placeholder for empty
    /// leaves in the Merkle tree.
    fn zero() -> E;
}
