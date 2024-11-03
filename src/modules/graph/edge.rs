use super::Node;

/// Represents an edge in the graph, connecting two nodes with a weight.
#[derive(Debug, Clone)]
pub struct Edge {
    pub from: Node,
    pub to: Node,
    pub weight: f64,
}

impl Edge {
    /// Creates a new edge between two nodes with the given weight.
    pub fn new(from: Node, to: Node, weight: f64) -> Self {
        Self { from, to, weight }
    }
}
