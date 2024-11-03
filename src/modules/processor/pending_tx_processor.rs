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
            debug!("Transaction {} is a pool interaction. Processing...", hash);

            let token_graph = config.app_state.token_graph.clone();
            let weth_address = Address::from_str(WETH)?;

            // Run SPFA starting from WETH
            let cycles = {
                let graph = token_graph.read().await;
                graph.find_negative_cycles_from_weth(weth_address)
            };
            let pool_detection_time = now.elapsed();
            let time_since_received = received_at.elapsed();

            if !cycles.is_empty() {
                for (i, path) in cycles.iter().enumerate() {
                    info!("Arbitrage opportunity {} detected: {:?}", i + 1, path);
                }
                // Proceed to compute potential profit and execute trade
            } else {
                debug!("No arbitrage opportunity found.");
            }

            info!(
                "Processed transaction: {} in {} ms (total time since received: {} ms)",
                tx.hash,
                pool_detection_time.as_millis(),
                time_since_received.as_millis()
            );
        }
    } else {
        debug!(
            "Transaction {} not found. It might have been dropped.",
            hash
        );
    }

    Ok(())
}
