use std::sync::Arc;

use crate::modules::config::Config;
use crate::utils::call_frame::get_call_frame;
use crate::utils::interaction::is_pool_interaction;
use alloy::primitives::FixedBytes;
use alloy::providers::Provider;

use log::{debug, info, warn};
use tokio::sync::mpsc::Receiver;

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
    pub async fn run(
        &self,
        pending_tx_receiver: &mut Receiver<FixedBytes<32>>,
    ) -> eyre::Result<()> {
        info!("Initializing PendingTransactionProcessor...");

        // TODO move the loading of the transaction to a seperate task
        while let Some(tx_hash) = pending_tx_receiver.recv().await {
            let provider = self.config.provider.clone();
            let config = self.config.clone();
            let current_block = *config.app_state.block_number.read().await;
            let current_base_fee = *config.app_state.next_block_base_fee.read().await;

            if current_block == 0 {
                warn!("Waiting for initial block number to be set");
                continue;
            }

            if current_base_fee == 0 {
                warn!("Waiting for initial base fee to be set");
                continue;
            }

            // Skip if transaction is already mined
            if let Some(_) = provider.get_transaction_receipt(tx_hash).await? {
                debug!("Transaction already mined, skipping: {}", tx_hash);
                continue;
            }

            // Fetch transaction details
            let tx = match provider.get_transaction_by_hash(tx_hash).await? {
                Some(tx) => tx,
                None => {
                    debug!("Transaction not found in mempool: {}", tx_hash);
                    continue;
                }
            };
            let current_block = *config.app_state.block_number.read().await;

            // Skip old transactions
            if let Some(block_number) = tx.block_number {
                if current_block.saturating_sub(block_number) > 5 || current_block == 0 {
                    debug!("Skipping outdated transaction: {}", tx.hash);
                    continue;
                }
            }

            // Skip transactions with insufficient gas
            if tx.max_fee_per_gas.unwrap_or(0) < u128::from(current_base_fee) {
                debug!("Insufficient max fee per gas, skipping: {}", tx.hash);
                continue;
            }

            // Analyze transaction call frame
            let call_frame = match get_call_frame(tx.clone(), provider.clone()).await {
                Ok(frame) => frame,
                Err(e) => {
                    debug!(
                        "Failed to get call frame for transaction {}: {}",
                        tx.hash, e
                    );
                    continue;
                }
            };

            // Check and process Uniswap V2 interactions
            let pools_map = config.app_state.pools_map.clone();
            if is_pool_interaction(&call_frame, &pools_map) {
                info!(
                    "Detected Uniswap V2 interaction in transaction: {}",
                    tx.hash
                );
                // Further processing
            }
        }

        Ok(())
    }
}
