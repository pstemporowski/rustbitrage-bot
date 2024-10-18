pub mod modules;
pub mod types;
pub mod utils;

use std::sync::Arc;
use tokio::task::JoinHandle;
use modules::{
    base_fee_updater::BaseFeeUpdater,
    blockchain_updater::BlockchainUpdater,
    config::Config,
    opportunity_finder::OpportunityFinder,
    transaction_executor::TransactionExecutor,
};
use futures::future::join_all;
use dotenv::dotenv;
use log::info;

#[tokio::main]
async fn main() {
    dotenv().ok();
    env_logger::init();

    let priv_key = std::env::var("PRIVATE_KEY").expect("PRIVATE_KEY must be set");
    let ws_url = std::env::var("WS_PROVIDER").expect("WS_PROVIDER must be set");

    info!("Connecting to WebSocket provider: {}", ws_url);

    let config = Arc::new(Config::new(ws_url, priv_key).await);
    let base_fee_updater = BaseFeeUpdater::new(config.clone());
    let updater = BlockchainUpdater::new(config.clone());
    let mut finder = OpportunityFinder::new(config.clone());
    let executor = TransactionExecutor::new(config.clone());

    let handles: Vec<JoinHandle<()>> = vec![
        tokio::spawn(async move { base_fee_updater.run().await }),
        tokio::spawn(async move { updater.run().await }),
        tokio::spawn(async move { finder.run().await }),
        tokio::spawn(async move { executor.run().await }),
    ];

    join_all(handles).await;
}