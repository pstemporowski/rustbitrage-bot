use super::{Edge, Node};
use alloy::primitives::{Address, U256};
use amms::amm::{AutomatedMarketMaker, AMM};
use dashmap::{DashMap, DashSet};
use eyre::Result;
use rayon::prelude::*;
use std::sync::Arc;
/// Represents a graph with nodes and edges.
#[derive(Debug, Clone)]
pub struct Graph {
    pub nodes: Arc<DashMap<Address, Node>>,
    pub edges: Arc<DashMap<(Address, Address), Edge>>,
}

impl Graph {
    /// Creates a new empty graph.
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(DashMap::new()),
            edges: Arc::new(DashMap::new()),
        }
    }

    /// Adds a node to the graph.
    pub fn add_node(&self, node: Node) {
        self.nodes.insert(node.address, node);
    }

    /// Adds an edge to the graph.
    pub fn add_edge(&self, edge: Edge) {
        self.edges
            .insert((edge.from.address, edge.to.address), edge);
    }

    /// Updates the weight of an edge or adds it if it doesn't exist.
    pub fn upsert_edge(&self, from: &Address, to: &Address, weight: f64) {
        let from_node = self.nodes.get(from).unwrap().clone();
        let to_node = self.nodes.get(to).unwrap().clone();
        let edge = Edge::new(from_node, to_node, weight);
        self.add_edge(edge);
    }

    /// Retrieves the edge between two nodes if it exists.
    pub fn get_edge(&self, from: &Address, to: &Address) -> Option<Edge> {
        self.edges.get(&(*from, *to)).map(|e| e.value().clone())
    }

    /// Returns all the nodes in the graph.
    pub fn get_nodes(&self) -> Vec<Node> {
        self.nodes
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Returns all the edges in the graph.
    pub fn get_edges(&self) -> Vec<Edge> {
        self.edges
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    pub fn update_by_pools(&self, pools: &Vec<AMM>) {
        for pool in pools {
            let tokens = pool.tokens();
            let token0 = tokens[0];
            let token1 = tokens[1];

            if let Ok(rate) = pool.calculate_price(token0, token1) {
                let weight = -rate.ln();
                self.upsert_edge(&token0, &token1, weight);
            }

            if let Ok(rate) = pool.calculate_price(token1, token0) {
                let weight = -rate.ln();
                self.upsert_edge(&token1, &token0, weight);
            }
        }
    }

    pub fn find_negative_cycles_from_weth(&self, weth_address: Address) -> Vec<Vec<Address>> {
        let node_addresses: Vec<Address> = self.nodes.iter().map(|entry| *entry.key()).collect();
        let num_nodes = node_addresses.len();

        // Shared distances and predecessors across threads
        let distances = Arc::new(DashMap::new());
        let predecessors = Arc::new(DashMap::new());

        // Initialize distances
        node_addresses.par_iter().for_each(|&address| {
            distances.insert(address, f64::INFINITY);
            predecessors.insert(address, None);
        });
        distances.insert(weth_address, 0.0);

        // Relax edges repeatedly
        for _ in 0..num_nodes - 1 {
            // Process edges in parallel
            self.edges.par_iter().for_each(|entry| {
                let edge = entry.value();
                let u = edge.from.address;
                let v = edge.to.address;
                let weight = edge.weight;

                let du = distances.get(&u).map(|r| *r).unwrap_or(f64::INFINITY);
                let dv = distances.get(&v).map(|r| *r).unwrap_or(f64::INFINITY);

                let new_distance = du + weight;
                if new_distance < dv {
                    distances.insert(v, new_distance);
                    predecessors.insert(v, Some(u));
                }
            });
        }

        // Check for negative-weight cycles
        let negative_cycles = DashSet::new();

        self.edges.par_iter().for_each(|entry| {
            let edge = entry.value();
            let u = edge.from.address;
            let v = edge.to.address;
            let weight = edge.weight;

            let du = distances.get(&u).map(|r| *r).unwrap_or(f64::INFINITY);
            let dv = distances.get(&v).map(|r| *r).unwrap_or(f64::INFINITY);

            if du + weight < dv {
                // Negative cycle detected
                if let Some(cycle) = self.build_negative_cycle(v, &predecessors) {
                    // Ensure the cycle starts and ends with WETH
                    if cycle.first() == Some(&weth_address) && cycle.last() == Some(&weth_address) {
                        negative_cycles.insert(cycle);
                    }
                }
            }
        });

        negative_cycles.into_iter().collect()
    }

    /// Simulates a swap on a given pool, updates the cloned graph,
    /// and checks for new arbitrage opportunities.
    ///
    /// # Arguments
    /// * `pool` - The pool on which to simulate the swap.
    /// * `token_in` - The address of the input token.
    /// * `amount_in` - The amount of the input token to swap.
    /// * `weth_address` - The WETH token address for arbitrage cycle detection.
    ///
    /// # Returns
    /// A vector of arbitrage cycles found after the simulation.
    pub fn simulate_swap_and_check_arbitrage(
        &self,
        pool: &AMM,
        token_in: Address,
        quote_token: Address,
        amount_in: U256,
        weth_address: Address,
    ) -> Result<Vec<Vec<Address>>> {
        // Clone the graph
        let cloned_graph = self.clone();

        // Clone the pool and simulate the swap
        let mut cloned_pool = pool.clone();

        // Perform the swap simulation using amms-rs
        // `simulate_swap` should modify the cloned_pool's state
        cloned_pool.simulate_swap_mut(token_in, quote_token, amount_in)?;

        // Update the edges in the cloned graph based on the new pool reserves
        cloned_graph.update_by_pools(&vec![cloned_pool]);

        // Run the negative cycle detection on the cloned graph
        let cycles = cloned_graph.find_negative_cycles_from_weth(weth_address);

        Ok(cycles)
    }

    fn build_negative_cycle(
        &self,
        start: Address,
        predecessors: &DashMap<Address, Option<Address>>,
    ) -> Option<Vec<Address>> {
        let mut current = start;
        let visited = DashMap::new();

        loop {
            if visited.contains_key(&current) {
                // Cycle detected
                let mut cycle_nodes = Vec::new();
                cycle_nodes.push(current);
                let mut node = predecessors.get(&current)?.clone().unwrap();

                while node != current {
                    cycle_nodes.push(node);
                    node = predecessors.get(&node)?.clone().unwrap();
                }
                cycle_nodes.push(current);
                cycle_nodes.reverse();
                return Some(cycle_nodes);
            }

            visited.insert(current, true);
            match predecessors.get(&current)?.clone() {
                Some(prev) => current = prev,
                None => break,
            }
        }

        None
    }
    /// Converts exchange rates to log-space weights for arbitrage detection.
    pub fn convert_rates_to_weights(&self) {
        for mut edge_entry in self.edges.iter_mut() {
            let edge = edge_entry.value_mut();
            // Convert the exchange rate to negative log for arbitrage detection
            edge.weight = -edge.weight.ln();
        }
    }
}
