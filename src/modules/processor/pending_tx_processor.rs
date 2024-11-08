use std::sync::Arc;

use crate::modules::config::Config;
use crate::types::pending_tx_hash::PendingTx;
use crate::utils::transaction::process_transaction;
use eyre::Result;
use log::{debug, info};

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

            let hash = pending_tx.hash;
            let provider = Arc::clone(&self.config.provider);

            let block_number = *self.config.app_state.block_number.read().await;
            let graph = self.config.app_state.token_graph.clone();
            let pools_map = self.config.app_state.pools_map.clone();

            tokio::spawn(async move {
                // Process the transaction asynchronously.
                if let Err(e) = process_transaction(
                    pending_tx,
                    provider,
                    block_number,
                    block_number,
                    &pools_map,
                    graph,
                )
                .await
                {
                    debug!("Error processing transaction {}: {}", hash, e);
                }
                // Drop the permit when done.
                drop(permit);
            });
        }

        Ok(())
    }
}
