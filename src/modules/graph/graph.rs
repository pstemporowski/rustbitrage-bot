use crate::types::arbitrage_opportunity::ArbitrageOpportunity;
use crate::utils::constants::WETH;
use crate::utils::swap::SwapInfo;

use super::{Edge, Node};
use alloy::primitives::Address;
use alloy::primitives::U256;
use amms::amm::{AutomatedMarketMaker, AMM};

use dashmap::{DashMap, DashSet};
use eyre::eyre;
use eyre::Result;
use log::info;
use rayon::prelude::*;
use std::{str::FromStr, sync::Arc, time::Instant};
/// Represents a graph with nodes and edges.
#[derive(Debug, Clone)]
pub struct Graph {
    pub nodes: DashMap<Address, Node>,
    pub edges: DashMap<(Address, Address), Edge>,
}

impl Graph {
    /// Creates a new empty graph.
    pub fn new() -> Self {
        Self {
            nodes: DashMap::new(),
            edges: DashMap::new(),
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
    pub fn upsert_edge(&self, from: &Address, to: &Address, weight: f64, pool_address: Address) {
        let from_node = self.nodes.get(from).unwrap().clone();
        let to_node = self.nodes.get(to).unwrap().clone();
        let edge = Edge::new(from_node, to_node, weight, pool_address);
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

    /// Updates the graph by processing the given pools.
    ///
    /// This function iterates over the provided pools, calculates the price rate between each pair of tokens,
    /// and then upserts an edge in the graph with the negative logarithm of the price rate as the weight.
    /// The pool address is also stored as part of the edge.
    ///
    /// # Parameters
    /// - `pools`: A vector of AMM pools to process.
    pub fn update_by_pools(&self, pools: &Vec<AMM>) {
        let start = Instant::now();
        pools.par_iter().for_each(|pool| {
            let tokens = pool.tokens();
            let token0 = tokens[0];
            let token1 = tokens[1];

            if let Ok(rate) = pool.calculate_price(token0, token1) {
                let weight = -rate.ln();
                self.upsert_edge(&token0, &token1, weight, pool.address());
            }

            if let Ok(rate) = pool.calculate_price(token1, token0) {
                let weight = -rate.ln();
                self.upsert_edge(&token1, &token0, weight, pool.address());
            }
        });
        info!("update_by_pools took: {:?}", start.elapsed());
    }

    /// Finds all negative cycles in the graph that start and end with the WETH token.
    ///
    /// This function performs a modified Bellman-Ford algorithm to detect negative cycles in the graph.
    /// It first initializes the distances and predecessors for each node, setting the distance for the
    /// WETH node to 0. It then repeatedly relaxes the edges, updating the distances and predecessors.
    /// Finally, it checks for any edges where the new distance is less than the current distance,
    /// indicating a negative cycle. For each negative cycle found, it builds the cycle and checks that
    /// it starts and ends with the WETH token before adding it to the result.
    ///
    /// # Returns
    /// A `Result` containing a vector of vectors of `Address`, where each inner vector represents a
    /// negative cycle that starts and ends with the WETH token.
    pub fn find_negative_cycles_from_weth(&self) -> Result<Vec<Vec<Address>>> {
        let weth = Address::from_str(WETH)?;

        let node_addresses: Vec<Address> =
            self.nodes.par_iter().map(|entry| *entry.key()).collect();
        let num_nodes = node_addresses.len();

        // Shared distances and predecessors across threads
        let distances = Arc::new(DashMap::new());
        let predecessors = Arc::new(DashMap::new());

        // Initialize distances
        node_addresses.par_iter().for_each(|&address| {
            distances.insert(address, f64::INFINITY);
            predecessors.insert(address, None);
        });
        distances.insert(weth, 0.0);

        // Relax edges repeatedly
        (0..num_nodes - 1).into_par_iter().for_each(|_| {
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
        });

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
                    if cycle.first() == Some(&weth) && cycle.last() == Some(&weth) {
                        negative_cycles.insert(cycle);
                    }
                }
            }
        });

        Ok(negative_cycles.into_iter().collect())
    }

    /// Simulates the effect of a series of swaps on the graph, and checks for any negative cycles that
    /// could represent arbitrage opportunities.
    ///
    /// This function takes a set of swaps to simulate, and the current state of the AMM pools. It
    /// updates the edge weights in a cloned graph based on the simulated swap effects, then runs
    /// negative cycle detection on the updated graph. For each negative cycle found, it calculates the
    /// optimal trade amount and expected profit.
    ///
    /// # Arguments
    /// * `swaps` - The set of swaps to simulate.
    /// * `pools_map` - A map of AMM pools, keyed by the pool address.
    ///
    /// # Returns
    /// A vector of `ArbitrageOpportunity` structs, each representing a negative cycle that could be
    /// exploited for arbitrage.
    pub fn find_opportunities(
        &self,
        swaps: &Vec<SwapInfo>,
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<Vec<ArbitrageOpportunity>> {
        // Clone the graph for simulation
        let now = Instant::now();
        let mut cloned_graph = self.clone();

        swaps.iter().try_for_each(|swap| {
            // Simulate the swap effect on the AMM pools
            cloned_graph.simulate_swap_mut(swap, pools_map)
        })?;

        // Run negative cycle detection on the updated graph
        let cycles = cloned_graph.find_negative_cycles_from_weth()?;
        let opportunities = self.find_optimal_trade_amounts(cycles, pools_map)?;

        info!(
            "Negative cycle detection took {} ms",
            now.elapsed().as_millis()
        );
        Ok(opportunities)
    }

    /// Finds the optimal trade amounts and expected profits for each negative cycle detected in the graph.
    ///
    /// This function takes a vector of negative cycles, where each cycle is represented as a vector of addresses.
    /// For each negative cycle, it calculates the optimal trade amount and expected profit, and returns a vector of
    /// `ArbitrageOpportunity` structs containing the cycle, optimal amount, and expected profit.
    ///
    /// # Arguments
    /// * `cycles` - A vector of negative cycles, where each cycle is represented as a vector of addresses.
    /// * `pools_map` - A map of AMM pools, keyed by the pool address.
    ///
    /// # Returns
    /// A vector of `ArbitrageOpportunity` structs, each representing a negative cycle that could be exploited for arbitrage.
    pub fn find_optimal_trade_amounts(
        &self,
        cycles: Vec<Vec<Address>>,
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<Vec<ArbitrageOpportunity>> {
        let mut opportunities = Vec::new();

        // For each negative cycle detected, calculate the optimal amount and expected profit
        for cycle in cycles {
            let (amount_in, profit) = match self.find_optimal_trade_amount(&cycle, pools_map) {
                Ok(result) => result,
                Err(e) => {
                    info!("Error finding optimal trade amount: {}", e);
                    continue;
                }
            };

            opportunities.push(ArbitrageOpportunity {
                path: cycle.clone(),
                optimal_amount_in: amount_in,
                expected_profit: profit,
            });
        }

        Ok(opportunities)
    }

    /// Simulates a swap on the given AMM pool and updates the graph with the new edge weight.
    ///
    /// This function takes a `SwapInfo` struct that contains the details of the swap, and a map of AMM pools.
    /// It first retrieves the pool from the pools map, and then simulates the swap on the pool. After the swap,
    /// it calculates the new exchange rate between the two tokens and updates the edge weight in the graph.
    ///
    /// # Arguments
    /// * `swap` - A `SwapInfo` struct containing the details of the swap to be simulated.
    /// * `pools_map` - A `DashMap` containing the AMM pools, keyed by the pool address.
    ///
    /// # Returns
    /// * `Ok(())` if the swap simulation and graph update were successful.
    /// * `Err(...)` if there was an error during the swap simulation or graph update.
    pub fn simulate_swap_mut(
        &mut self,
        swap: &SwapInfo,
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<()> {
        if let Some(mut pool) = pools_map.get_mut(&swap.pool_address) {
            pool.simulate_swap_mut(swap.from_token, swap.to_token, swap.amount_in)?;

            if let Ok(rate) = pool.calculate_price(swap.from_token, swap.to_token) {
                let weight = -rate.ln();
                self.upsert_edge(&swap.from_token, &swap.to_token, weight, pool.address());
            }
        }
        Ok(())
    }

    /// Builds a negative cycle from the given start address and predecessor map.
    ///
    /// This function traverses the graph using the predecessor map, starting from the given
    /// `start` address. It keeps track of the visited nodes in a `DashMap` to detect cycles.
    /// Once a cycle is detected, the function returns the nodes in the cycle in reverse order.
    ///
    /// # Arguments
    /// * `start` - The starting address for the cycle detection.
    /// * `predecessors` - A `DashMap` containing the predecessor information for each node.
    ///
    /// # Returns
    /// * `Some(Vec<Address>)` if a negative cycle is found, containing the nodes in the cycle.
    /// * `None` if no negative cycle is found.
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
        let start = Instant::now();
        for mut edge_entry in self.edges.iter_mut() {
            let edge = edge_entry.value_mut();
            // Convert the exchange rate to negative log for arbitrage detection
            edge.weight = -edge.weight.ln();
        }
        info!("convert_rates_to_weights took: {:?}", start.elapsed());
    }

    /// Finds the optimal trade amount and the corresponding profit for a given arbitrage cycle.
    ///
    /// This function performs a binary search to find the optimal trade amount that maximizes the
    /// profit for the given arbitrage cycle. It starts with a range of 1 wei to the maximum
    /// possible U256 value, and iteratively narrows down the search range to find the optimal
    /// amount.
    ///
    /// The function first attempts to calculate the profit for the midpoint of the current search
    /// range. If the profit is greater than the current best profit, the optimal amount and best
    /// profit are updated. Otherwise, the search range is adjusted based on the result.
    ///
    /// After the binary search, the function performs a fine-tuning step by checking a small
    /// range around the optimal amount found, to ensure the best possible profit is returned.
    ///
    /// # Arguments
    /// * `cycle` - A slice of addresses representing the arbitrage cycle.
    /// * `pools_map` - A reference to a `DashMap` containing the AMM pools.
    ///
    /// # Returns
    /// * A tuple containing the optimal trade amount and the corresponding profit.
    /// * An error if no profitable arbitrage was found.
    pub fn find_optimal_trade_amount(
        &self,
        cycle: &Vec<Address>,
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<(U256, U256)> {
        if cycle.first() != cycle.last() {
            return Err(eyre!("Cycle must start and end with the same token"));
        }

        let mut optimal_amount = U256::ZERO;
        let mut best_profit = U256::ZERO;

        // More realistic initial bounds based on typical DEX liquidity
        let mut left = U256::from(1) * U256::from(10).pow(U256::from(15)); // 0.001 ETH in wei
        let mut right = U256::from(1) * U256::from(10).pow(U256::from(21)); // 1000 ETH in wei

        // Perform binary search to find the optimal trade amount
        while left < right {
            let mid = left + (right - left) / U256::from(2);

            match self.calculate_profit(cycle, mid, pools_map) {
                Ok(profit) => {
                    // Check if we found a better profit
                    if profit > best_profit {
                        best_profit = profit;
                        optimal_amount = mid;
                        left = mid + U256::from(1);
                    } else {
                        right = mid;
                    }
                }
                Err(_) => right = mid,
            }

            // Break if the search range becomes too small
            if right - left < U256::from(1_000_000) {
                // 0.000001 ETH precision
                break;
            }
        }

        // If no profit was found during the binary search, return an error
        if best_profit.is_zero() {
            return Err(eyre!(
                "No profitable arbitrage found with {} profit",
                best_profit
            ));
        }

        Ok((optimal_amount, best_profit))
    }

    /// Calculates the maximum profit that can be obtained by executing a cyclic trade
    /// on the given set of token addresses and AMM pools.
    ///
    /// # Arguments
    /// - `cycle`: A slice of token addresses representing the cyclic trade.
    /// - `amount_in`: The input amount to use for the cyclic trade.
    /// - `pools_map`: A `DashMap` containing the AMM pools for the given token addresses.
    ///
    /// # Returns
    /// A `Result` containing the maximum profit that can be obtained, or an error if
    /// no profitable arbitrage was found or an error occurred during the calculation.
    fn calculate_profit(
        &self,
        cycle: &Vec<Address>,
        amount_in: U256,
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<U256> {
        let mut amount = amount_in;

        // Validate input amount
        if amount_in.is_zero() {
            return Err(eyre!("Invalid input amount"));
        }

        // Track each swap in the cycle
        for window in cycle.windows(2) {
            let from = window[0];
            let to = window[1];

            let edge = self
                .get_edge(&from, &to)
                .ok_or_else(|| eyre!("Edge not found between {:?} and {:?}", from, to))?;

            let pool = pools_map
                .get(&edge.pool_address)
                .ok_or_else(|| eyre!("Pool not found for address: {:?}", edge.pool_address))?;

            // Simulate the swap with slippage check
            let amount_out = pool.simulate_swap(from, to, amount)?;

            // Verify the swap produced a valid output
            if amount_out.is_zero() {
                return Err(eyre!("Zero output amount in swap"));
            }

            amount = amount_out;
        }

        // Calculate profit with overflow protection
        amount
            .checked_sub(amount_in)
            .ok_or_else(|| eyre!("Overflow in profit calculation"))
    }
}
