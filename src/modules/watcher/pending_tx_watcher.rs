use alloy::providers::Provider;
use eyre::Result;
use futures::StreamExt;
use log::{debug, info, warn};
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::sleep};

use crate::{modules::config::Config, types::pending_tx_hash::PendingTx};

/// Watches for pending transactions on the blockchain and forwards them to a channel
pub struct PendingTransactionWatcher {
    config: Arc<Config>,
}

impl PendingTransactionWatcher {
    /// Creates a new PendingTransactionWatcher instance
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Runs the watcher, monitoring for pending transactions and sending them to the provided channel
    ///
    /// # Arguments
    /// * `pending_tx_hash_sender` - Channel sender for forwarding pending transaction hashes
    ///
    /// # Returns
    /// * `Result<()>` - Success or error of the watcher operation
    pub async fn run(self, pending_tx_sender: mpsc::Sender<PendingTx>) -> Result<()> {
        let provider = self.config.provider.clone();

        // Wait for application initialization
        while !*self.config.app_state.is_initialized.read().await {
            sleep(Duration::from_secs(1)).await;
        }
        info!("PendingTransactionWatcher initialized and starting subscription to pending transactions");

        let sub = provider.subscribe_pending_transactions().await?;
        let mut stream = sub.into_stream();
        let sender = pending_tx_sender.clone();

        info!("Successfully established pending transaction stream");
        while let Some(tx_hash) = stream.next().await {
            debug!("New pending transaction detected [hash: {}]", tx_hash);
            let pending_tx = PendingTx::new(tx_hash);
            if let Err(e) = sender.send(pending_tx).await {
                warn!("Failed to forward pending transaction hash: {}", e);
                continue;
            }
        }

        warn!("Pending transaction stream terminated unexpectedly");
        Ok(())
    }
}
