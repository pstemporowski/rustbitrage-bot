use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use crate::modules::config::Config;
use crate::types::pending_tx_hash::PendingTx;
use crate::utils::call_frame::get_call_frame;
use crate::utils::constants::WETH;
use crate::utils::interaction::is_pool_interaction;
use alloy::primitives::Address;
use alloy::providers::{Provider, RootProvider};
use alloy::pubsub::PubSubFrontend;
use eyre::Result;
use log::{debug, info};
use petgraph::graph::NodeIndex;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use tokio::sync::mpsc::Receiver;
use tokio::sync::Semaphore;
use tokio::time::Instant;

/// Processes pending transactions from the mempool, specifically focusing on
/// Uniswap V2 interactions and related DeFi operations.
pub struct PendingTransactionProcessor {
    config: Arc<Config>,
}

impl PendingTransactionProcessor {
    /// Creates a new instance of PendingTransactionProcessor with the provided configuration.
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Main processing loop that handles incoming pending transactions.
    ///
    /// # Arguments
    /// * `pending_tx_receiver` - Channel receiver for pending transaction hashes
    ///
    /// # Returns
    /// * `eyre::Result<()>` - Result indicating success or failure of the processing
    pub async fn run(&self, pending_tx_receiver: &mut Receiver<PendingTx>) -> eyre::Result<()> {
        info!("Initializing PendingTransactionProcessor...");

        // Limit the number of concurrent tasks.
        let semaphore = Arc::new(Semaphore::new(100)); // Adjust the limit as needed.

        while let Some(pending_tx) = pending_tx_receiver.recv().await {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let provider = self.config.provider.clone();
            let config = self.config.clone();
            let hash = pending_tx.hash;

            tokio::spawn(async move {
                // Process the transaction asynchronously.
                if let Err(e) = process_transaction(pending_tx, provider, config).await {
                    debug!("Error processing transaction {}: {}", hash, e);
                }
                // Drop the permit when done.
                drop(permit);
            });
        }

        Ok(())
    }
}

/// Processes a single pending transaction, focusing on Uniswap V2 interactions and related DeFi operations.
///
/// # Arguments
/// * `tx_hash` - The hash of the pending transaction to process.
/// * `provider` - The Ethereum provider to use for fetching transaction data.
/// * `config` - The application configuration, including the current block number and base fee.
///
/// # Returns
/// * `eyre::Result<()>` - Result indicating success or failure of the processing.
async fn process_transaction(
    pending_tx: PendingTx,
    provider: Arc<RootProvider<PubSubFrontend>>,
    config: Arc<Config>,
) -> eyre::Result<()> {
    let now = Instant::now();
    let current_block = *config.app_state.block_number.read().await;
    let current_base_fee = *config.app_state.next_block_base_fee.read().await;
    let hash = pending_tx.hash;
    let received_at = pending_tx.received_at;

    // Early exit if block number or base fee is not set.
    if current_block == 0 || current_base_fee == 0 {
        debug!(
            "Block number or base fee not set. Skipping transaction {}",
            hash
        );
        return Ok(());
    }

    // Early filtering: Skip transactions with low gas price.
    // Fetching minimal transaction info to check gas price.
    if let Some(tx) = provider.get_transaction_by_hash(hash).await? {
        // If transaction does have a blocknumber, skip it as it's already mined
        if tx.block_number.is_some() {
            debug!("Transaction {} is already mined. Skipping.", hash);
            return Ok(());
        }

        if tx.max_fee_per_gas.unwrap_or(0) < u128::from(current_base_fee) {
            debug!(
                "Transaction {} has insufficient max fee per gas. Skipping.",
                hash
            );
            return Ok(());
        }
        // Analyze transaction call frame
        let call_frame = match get_call_frame(tx.clone(), provider.clone()).await {
            Ok(frame) => frame,
            Err(e) => {
                debug!(
                    "Failed to get call frame for transaction {}: {}",
                    tx.hash, e
                );
                return Ok(());
            }
        };

        // Check and process Uniswap V2 interactions
        let pools_map = config.app_state.pools_map.clone();
        if is_pool_interaction(&call_frame, &pools_map) {
            let cycle = find_best_cyclic_arbitrage(&config).await?;
            let pool_detection_time = now.elapsed();
            let time_since_received = received_at.elapsed();

            info!("Cycle: {:?}", cycle);

            info!(
                "Detected pool interaction in transaction: {} in {} ms (total time since received: {} ms)",
                tx.hash,
                pool_detection_time.as_millis(),
                time_since_received.as_millis()
            );
            // Further processing
        }
    } else {
        debug!(
            "Transaction {} not found. It might have been dropped.",
            hash
        );
    }

    Ok(())
}

/// Finds the best cyclic arbitrage opportunity in the token graph.
///
/// This function clones the token graph and token indices from the configuration, and then uses Dijkstra's algorithm to
/// find the shortest paths from the WETH node to all other nodes. It then checks for negative cycles, which indicate
/// potential arbitrage opportunities. If a negative cycle starting and ending at WETH is found, it is returned as the
/// best cyclic arbitrage.
///
/// # Arguments
/// * `config` - A reference to the application configuration.
///
/// # Returns
/// * `Result<Option<Vec<Address>>>` - An optional vector of token addresses representing the best cyclic arbitrage
///   opportunity, or `None` if no such opportunity was found.
pub async fn find_best_cyclic_arbitrage(config: &Arc<Config>) -> Result<Option<Vec<Address>>> {
    // Clone the token graph and indices for thread-safe operations
    let token_graph = config.app_state.token_graph.read().await.clone();
    let token_indices = config.app_state.token_indices.read().await.clone();

    // Get the node index for WETH
    let weth_address = Address::from_str(WETH).unwrap();
    let start_node = if let Some(index) = token_indices.get(&weth_address) {
        *index
    } else {
        return Ok(None); // WETH not in graph
    };

    // Initialize distances and predecessors
    let node_count = token_graph.node_count();
    let mut distances = vec![std::f64::INFINITY; node_count];
    let mut predecessors = vec![None; node_count];

    distances[start_node.index()] = 0.0;

    // Relax edges up to node_count - 1 times
    for _ in 0..node_count - 1 {
        let mut updated = false;

        // Parallel edge relaxation using Rayon
        let updates: Vec<(usize, f64, Option<NodeIndex>)> = token_graph
            .edge_indices()
            .collect::<Vec<_>>()
            .par_iter()
            .filter_map(|&edge_index| {
                let (source, target, weight) = token_graph
                    .edge_endpoints(edge_index)
                    .map(|(s, t)| (s, t, token_graph[edge_index]))
                    .unwrap();
                let u = source.index();
                let v = target.index();

                let alt = distances[u] + weight;
                if alt < distances[v] {
                    Some((v, alt, Some(source)))
                } else {
                    None
                }
            })
            .collect();

        // Apply updates sequentially
        for (v, alt, pred) in updates {
            distances[v] = alt;
            predecessors[v] = pred;
            updated = true;
        }

        // Early exit if no updates were made
        if !updated {
            break;
        }
    }

    // Check for negative cycles
    let mut arbitrage_cycle = None;

    // Collect edges that can be further relaxed (indicating a negative cycle)
    let negative_edges: Vec<_> = token_graph
        .edge_indices()
        .collect::<Vec<_>>()
        .into_par_iter()
        .filter_map(|edge_index| {
            let (source, target, weight) = token_graph
                .edge_endpoints(edge_index)
                .map(|(s, t)| (s, t, token_graph[edge_index]))
                .unwrap();
            let u = source.index();
            let v = target.index();

            if distances[u] + weight < distances[v] {
                Some(edge_index)
            } else {
                None
            }
        })
        .collect();

    // Attempt to find a negative cycle starting and ending at WETH
    for edge_index in negative_edges {
        let (source, _) = token_graph.edge_endpoints(edge_index).unwrap();
        let mut cycle = Vec::new();
        let mut visited = HashMap::new();
        let mut current = source;

        while !visited.contains_key(&current) {
            visited.insert(current, true);
            cycle.push(token_graph[current]);

            if let Some(pred_node) = predecessors[current.index()] {
                current = pred_node;
            } else {
                break;
            }
        }

        cycle.push(token_graph[current]); // Close the cycle

        // Check if the cycle starts and ends with WETH
        if cycle.first() == Some(&weth_address) && cycle.last() == Some(&weth_address) {
            arbitrage_cycle = Some(cycle);
            break;
        }
    }

    Ok(arbitrage_cycle)
}
