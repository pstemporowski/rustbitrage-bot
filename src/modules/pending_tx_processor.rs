use std::sync::Arc;

use crate::modules::config::Config;
use crate::utils::call_frame::get_call_frame;
use crate::utils::interaction::is_uniswap_v2_interaction;
use alloy::primitives::{Address, FixedBytes};
use alloy::providers::Provider;

use dashmap::DashSet;
use log::{debug, info, warn};
use tokio::sync::mpsc::Receiver;

pub struct PendingTransactionProcessor {
    config: Arc<Config>,
    uniswap_v2_addresses: Arc<DashSet<Address>>,
}

impl PendingTransactionProcessor {
    pub fn new(config: Arc<Config>, uniswap_v2_addresses: Arc<DashSet<Address>>) -> Self {
        Self {
            config,
            uniswap_v2_addresses,
        }
    }

    pub async fn run(
        &self,
        pending_tx_receiver: &mut Receiver<FixedBytes<32>>,
    ) -> eyre::Result<()> {
        info!("Starting PendingTransactionLoader...");

        // TODO move the loading of the transaction to a seperate task
        while let Some(tx_hash) = pending_tx_receiver.recv().await {
            let provider = self.config.provider.clone();
            let config = self.config.clone();
            let current_block = *config.app_state.block_number.read().await;
            let current_base_fee = *config.app_state.next_block_base_fee.read().await;

            if current_block == 0 {
                warn!("Waiting for block number to be set");
                continue;
            }

            if current_base_fee == 0 {
                warn!("Waiting for next block base fee to be set");
                continue;
            }

            if let Some(_) = provider.get_transaction_receipt(tx_hash).await? {
                debug!("Ignoring transaction as it has been mined: {}", tx_hash);
                continue;
            }

            let tx = match provider.get_transaction_by_hash(tx_hash).await? {
                Some(tx) => tx,
                None => {
                    continue;
                }
            };
            let current_block = *config.app_state.block_number.read().await;

            if let Some(block_number) = tx.block_number {
                if current_block.saturating_sub(block_number) > 5 || current_block == 0 {
                    debug!("Skipping old transaction: {}", tx.hash);
                    continue;
                }
            }

            if tx.max_fee_per_gas.unwrap_or(0) < u128::from(current_base_fee) {
                debug!(
                    "Skipping transaction due to low max fee per gas: {}",
                    tx.hash
                );
                continue;
            }

            // Retrieve the CallFrame
            let call_frame = match get_call_frame(tx.clone(), provider.clone()).await {
                Ok(frame) => frame,
                Err(_) => continue,
            };

            // Check for Uniswap V2 interaction
            if is_uniswap_v2_interaction(&call_frame, &self.uniswap_v2_addresses) {
                info!("Transaction {:?} interacts with Uniswap V2", tx.hash);
                // Further processing
            }
        }

        Ok(())
    }
}
