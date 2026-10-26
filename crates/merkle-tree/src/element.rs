use std::fmt::Debug;

/// A Merkle Tree element
pub trait Element: Clone + Debug + Default + Send + Sync {}

impl<T: Clone + Debug + Default + Send + Sync> Element for T {}
