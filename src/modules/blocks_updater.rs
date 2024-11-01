use super::config::Config;
use alloy::{
    eips::{calc_next_block_base_fee, eip1559::BaseFeeParams},
    providers::Provider,
    rpc::types::Block,
};
use eyre::Result;
use futures::StreamExt;
use log::{debug, info};
use std::sync::Arc;
use tokio::time::Instant;

pub struct BlocksUpdater {
    config: Arc<Config>,
}

impl BlocksUpdater {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run(&self) -> Result<()> {
        info!("Starting BlocksUpdater");

        let last_block_number = self.config.provider.get_block_number().await?;
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
                log::error!("Error handling block: {}", e);
            }
        }

        loop {
            let sub = self.config.provider.clone().subscribe_blocks().await?;
            let mut block_stream = sub.into_stream();

            debug!("Starting block stream");
            while let Some(block) = block_stream.next().await {
                if let Err(e) = self.handle_block(block).await {
                    log::error!("Error handling block: {}", e);
                }
            }
        }
    }
    async fn handle_block(&self, block: Block) -> Result<()> {
        let now = Instant::now();
        let block_number = block.header.number;
        let base_fee = self.calculate_next_block_base_fee(block);

        let mut lock = self.config.app_state.next_block_base_fee.write().await;
        *lock = base_fee;

        let mut lock = self.config.app_state.block_number.write().await;
        *lock = block_number;

        info!(
            "Calculated next base fee for block {} in {} µs",
            block_number,
            now.elapsed().as_micros()
        );

        Ok(())
    }

    fn calculate_next_block_base_fee(&self, block: Block) -> u64 {
        let gas_used = block.header.gas_used;
        let gas_limit = block.header.gas_limit;
        let base_fee_per_gas = block.header.base_fee_per_gas.unwrap_or(1);
        debug!("Received new block: {}", block.header.number);

        let base_fee = calc_next_block_base_fee(
            gas_used,
            gas_limit,
            base_fee_per_gas,
            BaseFeeParams::ethereum(),
        );
        debug!("New base fee: {}", base_fee);

        return base_fee;
    }
}
