use crate::{modules::processor::pending_tx_processor::Swap, utils::constants::WETH};

use super::{Edge, Node};
use alloy::primitives::Address;
use alloy::primitives::U256;
use amms::amm::{AutomatedMarketMaker, AMM};

use dashmap::{DashMap, DashSet};
use eyre::eyre;
use eyre::Result;
use log::warn;
use log::{error, info};
use rayon::prelude::*;
use std::{str::FromStr, sync::Arc, time::Instant};
/// Represents a graph with nodes and edges.
#[derive(Debug, Clone)]
pub struct Graph {
    pub nodes: DashMap<Address, Node>,
    pub edges: DashMap<(Address, Address), Edge>,
}

/// Represents an arbitrage opportunity with the optimal amount and expected profit.
pub struct ArbitrageOpportunity {
    pub path: Vec<Address>,
    pub optimal_amount_in: U256,
    pub expected_profit: U256,
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

    pub fn find_negative_cycles_from_weth(&self) -> Result<Vec<Vec<Address>>> {
        let start = Instant::now();
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

        info!("find_negative_cycles_from_weth took: {:?}", start.elapsed());
        Ok(negative_cycles.into_iter().collect())
    }

    pub fn simulate_swaps_and_check_arbitrage(
        &self,
        swaps: &Swap,
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<Vec<ArbitrageOpportunity>> {
        let start = Instant::now();
        // Clone the graph for simulation
        let cloned_graph = self.clone();

        let pools = &swaps.pools;
        let path = &swaps.path;
        let mut amount_in = swaps.amount_in;

        for (i, pool) in pools.iter().enumerate() {
            let mut pool = pool.clone();
            let token0 = path[i];

            let token1 = match path.get(i + 1) {
                Some(token) => *token,
                None => {
                    error!(
                        "Invalid path with {:?} pools, pairs {:?} at index {}",
                        pools, path, i
                    );
                    break;
                }
            };
            // Simulate the swap's effect on the pool's reserves
            amount_in = pool.simulate_swap_mut(token0, token1, amount_in)?;

            // Update the edge weights in the cloned graph based on the new reserves
            if let Ok(rate) = pool.calculate_price(token0, token1) {
                let weight = -rate.ln();
                cloned_graph.upsert_edge(&token0, &token1, weight, pool.address());
            }
        }
        // Run negative cycle detection on the updated graph
        let cycles = cloned_graph.find_negative_cycles_from_weth()?;

        let mut opportunities = Vec::new();

        // For each negative cycle detected, calculate the optimal amount and expected profit
        for cycle in cycles {
            let (amount_in, profit) =
                match cloned_graph.find_optimal_trade_amount(&cycle, pools_map) {
                    Ok(result) => result,
                    Err(e) => {
                        error!("Error finding optimal trade amount: {:?}", e);
                        continue;
                    }
                };

            opportunities.push(ArbitrageOpportunity {
                path: cycle,
                optimal_amount_in: amount_in,
                expected_profit: profit,
            });
        }

        info!(
            "simulate_swaps_and_check_arbitrage took: {:?}",
            start.elapsed()
        );
        Ok(opportunities)
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
        let start = Instant::now();
        for mut edge_entry in self.edges.iter_mut() {
            let edge = edge_entry.value_mut();
            // Convert the exchange rate to negative log for arbitrage detection
            edge.weight = -edge.weight.ln();
        }
        info!("convert_rates_to_weights took: {:?}", start.elapsed());
    }

    pub fn find_optimal_trade_amount(
        &self,
        cycle: &[Address],
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<(U256, U256)> {
        let start = Instant::now();
        // Initialize variables
        let mut optimal_amount = U256::ZERO;
        let mut best_profit = U256::ZERO;

        // Define search bounds
        let mut left = U256::from(1); // Start from 1 wei
        let mut right = U256::MAX; // Maximum possible U256 value

        // Perform binary search to find the optimal trade amount
        for _ in 0..64 {
            // Calculate the midpoint using a safer method
            let mid = left.saturating_add(right) / U256::from(2);

            // Break the loop if the search interval is too small
            if mid == left || mid == right {
                break;
            }

            // Attempt to calculate profit for the midpoint amount
            match self.calculate_profit(cycle, mid, pools_map) {
                Ok(profit) => {
                    if profit > best_profit {
                        best_profit = profit;
                        optimal_amount = mid;
                        left = mid;
                    } else {
                        right = mid;
                    }
                }
                Err(_) => {
                    // If profit calculation fails, adjust the right bound
                    right = mid;
                }
            }
        }

        // If no profit was found during the binary search, return an error
        if best_profit.is_zero() {
            return Err(eyre!("No profitable arbitrage found"));
        }

        // Fine-tune the optimal amount found
        let step = optimal_amount
            .checked_div(U256::from(100))
            .unwrap_or(U256::from(1));

        if !step.is_zero() {
            for i in -5i64..=5i64 {
                let offset = step
                    .checked_mul(U256::from(i.abs() as u64))
                    .unwrap_or(U256::ZERO);
                let test_amount = if i < 0 {
                    optimal_amount.checked_sub(offset).unwrap_or(U256::ZERO)
                } else {
                    optimal_amount.checked_add(offset).unwrap_or(U256::MAX)
                };

                // Skip if test_amount is zero
                if test_amount.is_zero() {
                    continue;
                }

                // Attempt to calculate profit for the test amount
                match self.calculate_profit(cycle, test_amount, pools_map) {
                    Ok(profit) => {
                        if profit > best_profit {
                            best_profit = profit;
                            optimal_amount = test_amount;
                        }
                    }
                    Err(e) => {
                        warn!("Error calculating profit for fine-tuning: {}", e);
                        continue;
                    }
                }
            }
        }

        info!("find_optimal_trade_amount took: {:?}", start.elapsed());
        Ok((optimal_amount, best_profit))
    }

    fn calculate_profit(
        &self,
        cycle: &[Address],
        amount_in: U256,
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<U256> {
        let start = Instant::now();
        let mut amount = amount_in;

        for window in cycle.windows(2) {
            let from = window[0];
            let to = window[1];
            let edge = match self.get_edge(&from, &to) {
                Some(edge) => edge,
                None => {
                    warn!("Edge not found between {:?} and {:?}", from, to);
                    break;
                }
            };
            let pool = pools_map
                .get(&edge.pool_address)
                .ok_or(eyre!("Pool not found"))?;

            amount = pool.simulate_swap(from, to, amount)?;
        }
        warn!("Found  profit: {} for amount: {}", amount, amount_in);
        // Profit is calculated as amount received minus amount spent
        let profit = amount.checked_sub(amount_in).ok_or_else(|| {
            warn!("Overflow occurred while calculating profit");
            eyre!("Overflow occurred while calculating profit")
        })?;

        if profit.is_zero() {
            return Err(eyre!("No profit found"));
        }

        info!("calculate_profit took: {:?}", start.elapsed());
        Ok(profit)
    }
}
