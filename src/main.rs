pub mod modules;
pub mod types;
pub mod utils;

use dotenv::dotenv;
use eyre::Result;
use futures::future::join_all;
use log::info;
use modules::{
    config::Config,
    processor::pending_tx_processor::PendingTransactionProcessor,
    watcher::{
        blocks_watcher::BlockWatcher, mempool_watcher::MempoolWatcher,
        pending_tx_watcher::PendingTransactionWatcher,
    },
};

use std::sync::Arc;
use tokio::{sync::mpsc, task::JoinHandle};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    env_logger::init();

    let ws_url = std::env::var("WS_PROVIDER").expect("WS_PROVIDER must be set");

    // let (execute_tx_sender, execute_tx_receiver) = mpsc::channel(512);
    // let (potential_opportunity_sender, potential_opportunity_receiver) = mpsc::channel(512);
    let (pending_tx_sender, mut pending_tx_receiver) = mpsc::channel(512);

    info!("Starting ArbitrageBot...");

    info!("Initializing Config...");
    let config = Arc::new(Config::new(ws_url).await?);

    info!("Initializing Modules...");
    let mem_pool_watcher = MempoolWatcher::new(config.clone());
    let block_watcher = BlockWatcher::new(config.clone());
    let tx_watcher = PendingTransactionWatcher::new(config.clone());
    let pending_tx_processor = PendingTransactionProcessor::new(config.clone());

    info!("Spawning Tasks...");
    let handles: Vec<JoinHandle<Result<()>>> = vec![
        tokio::spawn(async move { mem_pool_watcher.run().await }),
        tokio::spawn(async move { block_watcher.run().await }),
        tokio::spawn(async move { tx_watcher.run(pending_tx_sender).await }),
        tokio::spawn(async move { pending_tx_processor.run(&mut pending_tx_receiver).await }),
    ];

    join_all(handles).await;

    Ok(())
}
