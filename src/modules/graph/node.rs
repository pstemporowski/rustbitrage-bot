use alloy::primitives::Address;

/// Represents a node in the graph, holding a token address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Node {
    pub address: Address,
}

impl Node {
    /// Creates a new node with the given token address.
    pub fn new(address: Address) -> Self {
        Self { address }
    }
}
