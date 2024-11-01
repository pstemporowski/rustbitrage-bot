use alloy::{primitives::FixedBytes, providers::Provider};
use eyre::Result;
use futures::StreamExt;
use log::{debug, info};
use std::sync::Arc;
use tokio::sync::mpsc;

use super::config::Config;

pub struct PendingTransactionWatcher {
    config: Arc<Config>,
}

impl PendingTransactionWatcher {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run(self, pending_tx_hash_sender: mpsc::Sender<FixedBytes<32>>) -> Result<()> {
        info!("Starting PendingTransactionWatcher...");
        let provider = self.config.provider.clone();
        let sub = provider.subscribe_pending_transactions().await?;

        let mut stream = sub.into_stream();
        let sender = pending_tx_hash_sender.clone();

        debug!("Starting pending transaction stream");
        while let Some(tx_hash) = stream.next().await {
            debug!("Received new pending transaction: {}", tx_hash);
            if let Err(_) = sender.send(tx_hash).await {
                debug!("Failed to send pending transaction hash to channel");
                continue;
            }
        }
        Ok(())
    }

    //         if let Some(tx) = self.get_filtered_transaction(tx_hash).await {
    //             debug!("Found pending transaction: {}", tx.hash);
    //             let now = Instant::now();

    //             let current_block_number = self.config.wss.get_block_number().await.unwrap(); // TODO: Improve this
    //             let logs = match get_logs(&self.config.wss, &tx, BlockNumber::Number(current_block_number)).await {
    //                 Some(d) => d,
    //                 None => continue,
    //             };

    //             let significant_logs = {
    //                 let address_mapping = self.config.mapping.address_mapping.read().unwrap();
    //                 let pairs_mapping = self.config.mapping.pairs_mapping.read().unwrap();

    //                 logs.into_iter()
    //                     .filter_map(|log| {
    //                         let origin = log.address?;
    //                         let ptr = address_mapping.get(&origin)?;
    //                         if pairs_mapping.contains_key(ptr) {
    //                             Some(log)
    //                         } else {
    //                             None
    //                         }
    //                     })
    //                     .collect::<Vec<CallLogFrame>>()
    //             };

    //             if significant_logs.is_empty() {
    //                 debug!("No significant logs found for transaction: {}", tx.hash);
    //                 continue;
    //             }

    //             debug!("Found {} significant logs for transaction: {}", significant_logs.len(), tx.hash);
    //             let opportunity = PotentialOpportunity {
    //                 tx,
    //                 logs: significant_logs,
    //                 time: now,
    //             };

    //             match self.config.potential_opportunity_sender.try_send(opportunity) {
    //                 Ok(_) => (),
    //                 Err(TrySendError::Full(_)) => continue,
    //                 Err(TrySendError::Closed(_)) => break,
    //             }
    //         }
    //     }

    //     Ok(())
    // }

    // async fn get_filtered_transaction(&self, tx_hash: FixedBytes<32>) -> Option<Transaction> {
    //     let mut tx = match self.config.wss.get_transaction(tx_hash).await {
    //         Ok(Some(tx)) => tx,
    //         _ => return None,
    //     };

    //     if tx.to.is_none() {
    //         return None;
    //     }

    //     tx.from = tx.recover_from().ok()?;

    //     let base_fee = self.config.app_state.base_fee.read().await;
    //     if base_fee.is_zero() {
    //         warn!("Ignoring transaction {} as base_fee has not been set yet", tx.hash);
    //         return None;
    //     }

    //     if tx.max_fee_per_gas.unwrap_or(U256::from(0)) < *base_fee {
    //         debug!("Skipping transaction: {}", tx.hash);
    //         return None;
    //     }

    //     return Some(tx)
    // }
}
