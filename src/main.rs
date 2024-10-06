pub mod modules;
pub mod types;
pub mod utils;

use log::info;
use dotenv::dotenv;
use std::sync::Arc;
use tokio::task::JoinHandle;
use modules::config::Config;

use modules::base_fee_updater::BaseFeeUpdater;
use modules::opportunity_finder::OpportunityFinder;
use modules::blockchain_updater::BlockchainUpdater;
use modules::transaction_executor::TransactionExecutor;

#[tokio::main]
async fn main() {
    dotenv().ok();
    env_logger::init();

    let priv_key = std::env::var("PRIVATE_KEY").expect("PRIVATE_KEY must be set");
    let ws_url = std::env::var("WS_PROVIDER").expect("WS_PROVIDER must be set");
    
    info!("Connecting to WebSocket provider: {}", ws_url);
    info!("Starting bot with private key: {}", priv_key);

    let config = Arc::new(Config::new(ws_url, priv_key).await);
    let base_fee_updater = BaseFeeUpdater::new(config.clone());
    let updater = BlockchainUpdater::new(config.clone());
    let mut finder = OpportunityFinder::new(config.clone());
    let executor = TransactionExecutor::new(config.clone());

    let base_fee_updater_handle: JoinHandle<()> = tokio::spawn(async move { base_fee_updater.run().await });
    let updater_handle: JoinHandle<()> = tokio::spawn(async move { updater.run().await });
    let finder_handle: JoinHandle<()> = tokio::spawn(async move { finder.run().await });
    let executor_handle: JoinHandle<()> = tokio::spawn(async move { executor.run().await });

    tokio::try_join!(updater_handle, finder_handle, executor_handle, base_fee_updater_handle)
        .expect("One of the tasks failed");
}