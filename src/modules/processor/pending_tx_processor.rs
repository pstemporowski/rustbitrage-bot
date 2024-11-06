use std::sync::Arc;

use crate::modules::config::Config;
use crate::types::pending_tx_hash::PendingTx;
use crate::utils::call_frame::get_call_frame;
use crate::utils::interaction::{decode_swap_function, get_pool_interactions};
use alloy::primitives::{Address, Uint};
use alloy::providers::{Provider, RootProvider};
use alloy::pubsub::PubSubFrontend;
use amms::amm::AMM;
use eyre::Result;
use log::{debug, info, warn};

use tokio::sync::mpsc::Receiver;
use tokio::sync::Semaphore;
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
    pub async fn run(&self, pending_tx_receiver: &mut Receiver<PendingTx>) -> Result<()> {
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

pub struct Swap {
    pub path: Vec<Address>,
    pub amount_in: Uint<256, 4>,
    pub pools: Vec<AMM>,
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
        let mut interacted_pools = Vec::new();
        debug!("Analyzing pool interactions for transaction {}", hash);
        get_pool_interactions(&call_frame, &pools_map, &mut interacted_pools);
        if !interacted_pools.is_empty() {
            info!("Transaction {} is a pool interaction. Processing...", hash);
            debug!("Found {} interacting pools", interacted_pools.len());

            if let Some(swap_info) = decode_swap_function(&tx)? {
                info!(
                    "Successfully decoded swap function for transaction {}",
                    hash
                );
                // Prepare the swaps struct
                let swaps = Swap {
                    path: swap_info.path,
                    amount_in: swap_info.amount_in,
                    pools: interacted_pools.clone(),
                };

                // Simulate swaps and check for arbitrage opportunities
                let pools_map = config.app_state.pools_map.clone();
                let opportunities = {
                    let token_graph = config.app_state.token_graph.clone();
                    let graph = token_graph.read().await;

                    graph
                        .simulate_swaps_and_check_arbitrage(&swaps, &pools_map)
                        .unwrap_or_else(|e| {
                            warn!(
                                "Failed to find arbitrage opportunities after simulation: {}",
                                e
                            );
                            Vec::new()
                        })
                };

                opportunities.iter().for_each(|op| {
                    info!(
                        "Arbitrage opportunity: {} -> {} with profit of {} ETH",
                        op.path[0],
                        op.path[op.path.len() - 1],
                        op.expected_profit
                    );
                });

                let time_since_received = received_at.elapsed();

                info!(
                    "Processed transaction: {} in {} ms",
                    tx.hash,
                    time_since_received.as_millis()
                );
            } else {
                debug!("Failed to decode swap function for transaction {}", hash);
            }
        } else {
            debug!("No pool interactions found for transaction {}", hash);
        }
    } else {
        debug!(
            "Transaction {} not found. It might have been dropped.",
            hash
        );
    }

    Ok(())
}
