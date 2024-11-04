use crate::utils::constants::MAX_ITERATIONS;
use crate::{modules::processor::pending_tx_processor::Swap, utils::constants::WETH};
use rug::{ops::Pow, Float, Integer};

use super::{Edge, Node};
use alloy::primitives::U256;
use alloy::primitives::{Address, Uint};
use amms::amm::{AutomatedMarketMaker, AMM};

use dashmap::{DashMap, DashSet};
use eyre::eyre;
use eyre::Result;
use log::{debug, error};
use rayon::prelude::*;
use std::{str::FromStr, sync::Arc};
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
    pub expected_profit: f64,
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
    }

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
    /// Simulates the swaps on the cloned graph and checks for new arbitrage opportunities.
    ///
    /// # Arguments
    /// * `swaps` - A vector of swaps to simulate, each containing the token addresses and swap amounts.
    ///
    /// # Returns
    /// * `Result<Vec<Vec<Address>>>` - A list of arbitrage cycles found after simulation.
    ///
    /// # Notes
    /// This method clones the current graph to avoid mutating the original graph used by other processes.
    pub fn simulate_swaps_and_check_arbitrage(
        &self,
        swaps: &Swap,
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<Vec<ArbitrageOpportunity>> {
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
            if let Ok((optimal_amount_in, expected_profit)) =
                cloned_graph.find_optimal_trade_amount(&cycle, pools_map)
            {
                opportunities.push(ArbitrageOpportunity {
                    path: cycle,
                    optimal_amount_in,
                    expected_profit,
                });
            } else {
                debug!(
                    "Failed to calculate optimal trade amount for cycle {:?}",
                    cycle
                );
            }
        }

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
        for mut edge_entry in self.edges.iter_mut() {
            let edge = edge_entry.value_mut();
            // Convert the exchange rate to negative log for arbitrage detection
            edge.weight = -edge.weight.ln();
        }
    }

    /// Finds the optimal trade amount and calculates the potential profit for a given arbitrage cycle.
    ///
    /// # Arguments
    /// * `cycle` - A vector of `Address` representing the arbitrage cycle.
    ///
    /// # Returns
    /// * `Result<(U256, f64)>` - A tuple containing the optimal input amount and the expected profit.
    ///
    /// # Notes
    /// This method uses an analytical approach for cycles involving Uniswap V2 pools.
    /// For more complex cases, it falls back to a numerical method.
    pub fn find_optimal_trade_amount(
        &self,
        cycle: &[Address],
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<(U256, f64)> {
        // Check if all pools in the cycle are Uniswap V2 for analytical solution
        let are_all_uniswap_v2 = cycle.windows(2).all(|pair| {
            if let Some(edge) = self.get_edge(&pair[0], &pair[1]) {
                matches!(
                    pools_map
                        .get(&edge.pool_address)
                        .expect("Pool not found")
                        .value(),
                    AMM::UniswapV2Pool(_)
                )
            } else {
                false
            }
        });

        if are_all_uniswap_v2 {
            // Use analytical solution
            self.calculate_optimal_amount_analytically(cycle, pools_map)
        } else {
            // Use numerical method
            self.calculate_optimal_amount_numerically(cycle, pools_map);
        }
    }
    /// Calculates the optimal amount analytically for Uniswap V2 cycles.
    pub fn calculate_optimal_amount_analytically(
        &self,
        cycle: &[Address],
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<(U256, f64)> {
        // Constants for fee calculations (assuming Uniswap V2 0.3% fee)
        let fee_numerator = U256::from(997);
        let fee_denominator = U256::from(1000);

        // Initialize numerator and denominator for the optimal amount formula
        let mut numerator = U256::from(1);
        let mut denominator = U256::ZERO;

        // Iterate over the cycle pairs
        for window in cycle.windows(2) {
            let from = window[0];
            let to = window[1];
            let edge = self.get_edge(&from, &to).ok_or(eyre!("Edge not found"))?;
            let pool = pools_map
                .get(&edge.pool_address)
                .ok_or(eyre!("Pool not found"))?;

            if let AMM::UniswapV2Pool(uniswap_pool) = pool.value() {
                let (reserve_in, reserve_out) = if uniswap_pool.token_a == from {
                    (uniswap_pool.reserve_0, uniswap_pool.reserve_1)
                } else {
                    (uniswap_pool.reserve_1, uniswap_pool.reserve_0)
                };

                // Fee-adjusted reserves
                let fee_adjusted_reserve_in = reserve_in * fee_numerator;
                let fee_adjusted_reserve_out = reserve_out * fee_denominator;

                // Update numerator and denominator for the optimal amount formula
                numerator = numerator * fee_adjusted_reserve_in;

                if denominator.is_zero() {
                    denominator = fee_adjusted_reserve_out;
                } else {
                    denominator = denominator * fee_adjusted_reserve_out;
                }
            } else {
                return Err(eyre!("Non-Uniswap V2 pool found in analytical method"));
            }
        }

        if denominator.is_zero() {
            return Err(eyre!("Denominator is zero in analytical calculation"));
        }

        // Optimal amount in = numerator / denominator
        let optimal_amount_in = numerator
            .checked_div(denominator)
            .ok_or(eyre!("Division error"))?;

        // Calculate expected profit
        let profit = self.calculate_profit(cycle, optimal_amount_in, pools_map)?;

        // Convert U256 profit to f64 for expected_profit
        let expected_profit = profit.as_u128() as f64 / 1e18;

        Ok((optimal_amount_in, expected_profit))
    }

    /// Calculates the optimal amount using a numerical method.
    fn calculate_optimal_amount_numerically(
        &self,
        cycle: &[Address],
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<(U256, f64)> {
        let mut amount_in = Uint::<256, 4>::from(1_000_000u128); // Start with a reasonable guess
        let mut max_profit = Uint::<256, 4>::ZERO;
        let mut optimal_amount_in = amount_in;

        for _ in 0..MAX_ITERATIONS {
            let profit = self.calculate_profit(cycle, amount_in, pools_map)?;
            if profit > max_profit {
                max_profit = profit;
                optimal_amount_in = amount_in;
                amount_in *= Uint::<256, 4>::from(2u8); // Double the amount for next iteration
            } else {
                break;
            }
        }

        Ok((optimal_amount_in, max_profit.into()))
    }
    /// Simulates the cycle with a given amount in and calculates the profit.
    fn calculate_profit(
        &self,
        cycle: &[Address],
        amount_in: U256,
        pools_map: &DashMap<Address, AMM>,
    ) -> Result<U256> {
        let mut amount = amount_in;

        for window in cycle.windows(2) {
            let from = window[0];
            let to = window[1];
            let edge = self.get_edge(&from, &to).ok_or(eyre!("Edge not found"))?;
            let pool = pools_map
                .get(&edge.pool_address)
                .ok_or(eyre!("Pool not found"))?;

            amount = pool.simulate_swap(from, to, amount)?;
        }

        // Profit is calculated as amount received minus amount spent
        let profit = amount.checked_sub(amount_in).ok_or(eyre!("No profit"))?;
        Ok(profit)
    }
}
