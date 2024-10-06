use std::sync::Arc;

use super::config::Config;


pub struct TransactionExecutor {
    config: Arc<Config>,
}

impl TransactionExecutor {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run(&self) {
        loop {
            // Update blockchain data
            // Update shared state
        }
    }
}