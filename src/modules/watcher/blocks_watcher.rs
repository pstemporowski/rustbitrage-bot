use alloy::{
    eips::{calc_next_block_base_fee, eip1559::BaseFeeParams},
    providers::Provider,
    rpc::types::Block,
};
use eyre::Result;
use futures::StreamExt;
use log::{debug, info};
use std::{sync::Arc, time::Duration};
use tokio::time::{sleep, Instant};

use crate::modules::config::Config;

/// BlocksUpdater monitors new blocks and calculates the next block's base fee
pub struct BlockWatcher {
    config: Arc<Config>,
}

impl BlockWatcher {
    /// Creates a new BlocksUpdater instance with the provided configuration
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Runs the BlocksUpdater service, which continuously processes new blocks and updates the application state with the calculated next block base fee.
    ///
    /// This function first waits for the application to be initialized, then fetches the latest block and processes it. It then starts a continuous loop that subscribes to new blocks, processes them, and handles any errors that occur.
    pub async fn run(&self) -> Result<()> {
        // Wait for application initialization
        while !*self.config.app_state.is_initialized.read().await {
            debug!("Waiting for application initialization...");
            sleep(Duration::from_secs(1)).await;
        }
        info!("BlocksUpdater initialized and starting subscription to new blocks");

        // Process the latest block first
        let last_block_number = self.config.provider.get_block_number().await?;
        info!("Fetching initial block at height {}", last_block_number);

        let last_block = self
            .config
            .provider
            .get_block(
                last_block_number.into(),
                alloy::rpc::types::BlockTransactionsKind::Full,
            )
            .await?;

        if let Some(block) = last_block {
            if let Err(e) = self.handle_block(block).await {
                log::error!("Failed to process initial block: {}", e);
            }
        }

        // Start continuous block monitoring
        loop {
            let sub = self.config.provider.clone().subscribe_blocks().await?;
            let mut block_stream = sub.into_stream();

            debug!("Initialized block subscription stream");
            while let Some(block) = block_stream.next().await {
                if let Err(e) = self.handle_block(block).await {
                    log::error!("Failed to process incoming block: {}", e);
                }
            }
            info!("Block stream ended, attempting to reconnect...");
        }
    }

    /// Handles a new block by calculating the next block's base fee and updating the application state.
    ///
    /// This function is called whenever a new block is received from the Ethereum node. It calculates the next block's base fee using the EIP-1559 formula, and then updates the application state with the new base fee and block number.
    ///
    /// # Arguments
    /// * `block` - The new block to be processed.
    ///
    /// # Returns
    /// A `Result` indicating whether the block was processed successfully.
    async fn handle_block(&self, block: Block) -> Result<()> {
        let now = Instant::now();
        let block_number = block.header.number;
        let base_fee = self.calculate_next_block_base_fee(block);

        // Update application state with new base fee
        let mut lock = self.config.app_state.next_block_base_fee.write().await;
        *lock = base_fee;

        // Update current block number
        let mut lock = self.config.app_state.block_number.write().await;
        *lock = block_number;

        info!(
            "Block {}: Calculated next base fee {} gwei in {} µs",
            block_number,
            base_fee / 1_000_000_000,
            now.elapsed().as_micros()
        );

        Ok(())
    }

    /// Calculates the next block's base fee using the EIP-1559 formula.
    ///
    /// This function takes the current block's gas used, gas limit, and base fee per gas,
    /// and calculates the next block's base fee according to the EIP-1559 formula.
    ///
    /// # Arguments
    /// * `gas_used` - The amount of gas used in the current block.
    /// * `gas_limit` - The gas limit for the current block.
    /// * `base_fee_per_gas` - The base fee per gas in the current block.
    ///
    /// # Returns
    /// The calculated base fee for the next block.
    fn calculate_next_block_base_fee(&self, block: Block) -> u64 {
        let gas_used = block.header.gas_used;
        let gas_limit = block.header.gas_limit;
        let base_fee_per_gas = block.header.base_fee_per_gas.unwrap_or(1);
        debug!(
            "Processing block {}: gas_used={}, gas_limit={}, current_base_fee={}",
            block.header.number, gas_used, gas_limit, base_fee_per_gas
        );

        let base_fee = calc_next_block_base_fee(
            gas_used,
            gas_limit,
            base_fee_per_gas,
            BaseFeeParams::ethereum(),
        );
        debug!("Calculated next block base fee: {} wei", base_fee);

        return base_fee;
    }
}
