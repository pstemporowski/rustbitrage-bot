use std::{fs::File, io::BufReader, sync::Arc};

use alloy::{
    primitives::{address, Address},
    providers::RootProvider,
    pubsub::PubSubFrontend,
};
use amms::{
    amm::{factory::Factory, uniswap_v2::factory::UniswapV2Factory, AutomatedMarketMaker, AMM},
    sync::{checkpoint::Checkpoint, sync_amms},
};
use dashmap::DashSet;
use eyre::{Ok, Result};
use log::{debug, info};
use tokio::time::Instant;

use super::config::Config;

pub struct MempoolWatcher {
    config: Arc<Config>,
}

impl MempoolWatcher {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run(self) -> Result<Arc<DashSet<Address>>> {
        info!("Starting MempoolWatcher");
        let provider = self.config.provider.clone();
        let now = Instant::now();

        let checkpoint_path = ".cfmms-checkpoint.json";

        let amms = self
            .sync_or_load_amms(provider, checkpoint_path, true)
            .await?;

        // Create DashSet for UniswapV2 pool addresses
        let uniswap_v2_addresses = Arc::new(DashSet::new());

        // Populate DashSet with pool addresses
        for amm in amms {
            uniswap_v2_addresses.insert(amm.address());
        }

        info!(
            "Initialized {} UniswapV2 pool addresses in {} ms",
            uniswap_v2_addresses.len(),
            now.elapsed().as_millis()
        );

        Ok(uniswap_v2_addresses)
    }

    async fn sync_or_load_amms(
        self,
        provider: Arc<RootProvider<PubSubFrontend>>,
        checkpoint_path: &str,
        load_from_file: bool,
    ) -> Result<Vec<AMM>> {
        let now = Instant::now();

        // Load AMMs from file if available
        if load_from_file {
            let amms_vec = self.load_amms_from_file(checkpoint_path)?;
            info!(
                "Loaded {} AMMs from file in {} ms",
                amms_vec.len(),
                now.elapsed().as_millis()
            );
            return Ok(amms_vec);
        }

        // Sync AMMs from factory
        let factories = vec![
            // Add UniswapV2
            Factory::UniswapV2Factory(UniswapV2Factory::new(
                address!("5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f"),
                2638438,
                300,
            )),
        ];
        let (amms, _) = sync_amms(factories, provider, Some(checkpoint_path), 1000).await?;
        info!(
            "Synced {} AMMs in {} ms",
            amms.len(),
            now.elapsed().as_millis()
        );

        Ok(amms)
    }

    fn load_amms_from_file(&self, path: &str) -> Result<Vec<AMM>> {
        debug!("Loading AMMs from file: {}", path);
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let data: Checkpoint = serde_json::from_reader(reader)?;

        Ok(data.amms)
    }
}
