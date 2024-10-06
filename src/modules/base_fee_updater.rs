use std::sync::Arc;
use log::{info, error, debug};
use ethers::prelude::*;

use super::config::Config;

pub struct BaseFeeUpdater {
    config: Arc<Config>,
}

impl BaseFeeUpdater {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run(&self) {
        loop {
            let mut block_stream = match self.config.wss.subscribe_blocks().await {
                Ok(stream) => stream,
                Err(e) => {
                    error!("Failed to create new block stream: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            debug!("Starting block stream");

            while let Some(block) = block_stream.next().await {
                debug!("Received new block: {}", block.number.unwrap());

                let base_fee = self.calculate_next_block_base_fee(&block);
                info!("Current base fee: {}", base_fee);

                let mut lock = self.config.app_state.base_fee.write().await;
                *lock = base_fee
            }
        }
    }

    fn calculate_next_block_base_fee(&self, block: &Block<TxHash>) -> U256 {
        // Get the block base fee per gas
        let current_base_fee_per_gas = block.base_fee_per_gas.unwrap_or_default();

        // Get the mount of gas used in the block
        let current_gas_used = block.gas_used;

        let current_gas_target = block.gas_limit / 2;

        if current_gas_used == current_gas_target {
            current_base_fee_per_gas
        } else if current_gas_used > current_gas_target {
            let gas_used_delta = current_gas_used - current_gas_target;
            let base_fee_per_gas_delta =
                current_base_fee_per_gas * gas_used_delta / current_gas_target / 8;

            return current_base_fee_per_gas + base_fee_per_gas_delta;
        } else {
            let gas_used_delta = current_gas_target - current_gas_used;
            let base_fee_per_gas_delta =
                current_base_fee_per_gas * gas_used_delta / current_gas_target / 8;

            return current_base_fee_per_gas - base_fee_per_gas_delta;
        }
    }

  }
