use std::sync::Arc;

use log::info;

use super::config::Config;


pub struct OpportunityFinder {
    config: Arc<Config>,
}

impl OpportunityFinder {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run(&mut self) {
        loop {
            if let Some(tx) = self.config.opportunity_tx_receiver.lock().await.recv().await {
                // Process the OpportunityTx
                info!("Found opportunity: {}", tx.tx.hash);
            }
        }
    }
}