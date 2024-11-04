use crate::modules::config::Config;
use crate::modules::graph::Node;
use crate::utils::constants::{
    MIN_USD_FACTORY_THRESHOLD, TESTING_CHECKPOINT_PATH, USDC, WETH, WETH_USDC_PAIR, WETH_VALUE,
};
use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, RootProvider};
use alloy::pubsub::PubSubFrontend;
use amms::sync::checkpoint::Checkpoint;
use amms::{
    amm::{
        factory::Factory,
        uniswap_v2::{factory::UniswapV2Factory, UniswapV2Pool},
        AutomatedMarketMaker, AMM,
    },
    filters::value::filter_amms_below_usd_threshold,
    state_space::StateSpaceManager,
    sync::{checkpoint::sync_amms_from_checkpoint, sync_amms},
};
use eyre::Result;
use futures::future::join_all;
use log::{debug, info};
use rayon::prelude::*;
use std::fs::File;
use std::{path::Path, str::FromStr, sync::Arc};
use tokio::{sync::mpsc::Receiver, time::Instant};

/// Main struct responsible for watching mempool and managing AMM state
pub struct MempoolWatcher {
    config: Arc<Config>,
}

impl MempoolWatcher {
    /// Creates a new MempoolWatcher instance with the provided configuration
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Runs the MempoolWatcher service, which is responsible for watching the mempool and managing AMM state changes.
    pub async fn run(&self) -> Result<()> {
        info!("Starting MempoolWatcher service");
        let now = Instant::now();
        let checkpoint_path = ".cfmms-checkpoint.json";

        let provider = self.config.provider.clone();
        let pools_map = self.config.app_state.pools_map.clone();

        debug!("Starting AMM synchronization process");
        let (amms, latest_synced_block) = self
            .get_amms_and_latest_block(provider.clone(), checkpoint_path)
            .await?;

        self.initialize_pools(&amms, &pools_map);
        info!(
            "Pool initialization completed: {} pools in {} ms",
            pools_map.len(),
            now.elapsed().as_millis()
        );

        self.build_token_graph().await?;
        info!("Token graph construction completed");
        let mut is_initialized = self.config.app_state.is_initialized.write().await;
        *is_initialized = true;
        drop(is_initialized); // Explicitly drop the write lock
        info!("MempoolWatcher initialization completed");

        let state_change_receiver = self
            .subscribe_pools(provider.clone(), &amms, latest_synced_block)
            .await?;

        self.process_state_changes(provider, &pools_map, state_change_receiver)
            .await?;
        Ok(())
    }
    async fn process_state_changes(
        &self,
        provider: Arc<RootProvider<PubSubFrontend>>,
        pools_map: &Arc<dashmap::DashMap<Address, AMM>>,
        mut state_change_receiver: Receiver<Vec<Address>>,
    ) -> Result<()> {
        info!("MempoolWatcher is now monitoring for state changes");
        while let Some(changed_addresses) = state_change_receiver.recv().await {
            info!("Processing {} state changes", changed_addresses.len());
            let now = Instant::now();

            let updated_pools = self
                .sync_modified_pools(changed_addresses, provider.clone(), pools_map)
                .await;

            if !updated_pools.is_empty() {
                self.update_token_graph(&updated_pools).await?;
                info!(
                    "Token graph updated with {} modified pools in {} ms",
                    updated_pools.len(),
                    now.elapsed().as_millis()
                );
            }
        }
        Ok(())
    }

    fn initialize_pools(&self, amms: &[AMM], pools_map: &Arc<dashmap::DashMap<Address, AMM>>) {
        debug!("Initializing pool map with {} AMMs", amms.len());
        amms.par_iter().for_each(|amm| {
            pools_map.insert(amm.address(), amm.clone());
        });
    }

    async fn sync_modified_pools(
        &self,
        changed_addresses: Vec<Address>,
        provider: Arc<RootProvider<PubSubFrontend>>,
        pools_map: &Arc<dashmap::DashMap<Address, AMM>>,
    ) -> Vec<AMM> {
        let updated_pools_futures: Vec<_> = changed_addresses
            .into_iter()
            .filter_map(|addr| {
                pools_map.get(&addr).map(|entry| {
                    let mut pool = entry.value().clone();
                    let provider_clone = provider.clone();
                    tokio::spawn(async move {
                        match pool.sync(provider_clone).await {
                            Ok(_) => Some(pool),
                            Err(e) => {
                                debug!("Error syncing pool {:?}: {:?}", addr, e);
                                None
                            }
                        }
                    })
                })
            })
            .collect();

        if updated_pools_futures.is_empty() {
            return vec![];
        }

        debug!(
            "Syncing {} modified pools concurrently",
            updated_pools_futures.len()
        );

        join_all(updated_pools_futures)
            .await
            .into_iter()
            .filter_map(|r| r.ok().and_then(|x| x))
            .collect()
    }
    async fn get_amms_and_latest_block(
        &self,
        provider: Arc<RootProvider<PubSubFrontend>>,
        checkpoint_path: &str,
    ) -> Result<(Vec<AMM>, u64)> {
        let factories = vec![Factory::UniswapV2Factory(UniswapV2Factory::new(
            Address::from_str("5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f").unwrap(),
            10000835,
            300,
        ))];

        if self.config.is_testing {
            let cp = self.load_testing_checkpoint()?;
            let latest_block_number = provider.get_block_number().await?;
            Ok((cp.amms, latest_block_number))
        } else {
            let (amms, latest_synced_block) = self
                .sync_or_load_amms(provider.clone(), factories.clone(), checkpoint_path)
                .await?;

            info!("Filtering AMMs below USD threshold");
            let amms_before_filtering = amms.len();
            let amms = self
                .filter_amms(
                    amms,
                    factories.clone(),
                    MIN_USD_FACTORY_THRESHOLD,
                    provider.clone(),
                )
                .await?;

            self.write_testing_checkpoint(&factories, &amms, latest_synced_block)?;

            let amms_after_filtering = amms.len();
            info!("AMMs after filtering: {}", amms_after_filtering);
            info!("AMMs before filtering: {}", amms_before_filtering);
            Ok((amms, latest_synced_block))
        }
    }

    fn write_testing_checkpoint(
        &self,
        factories: &Vec<Factory>,
        amms: &Vec<AMM>,
        latest_synced_block: u64,
    ) -> Result<()> {
        let cp = Checkpoint::new(
            Instant::now().elapsed().as_secs() as usize,
            latest_synced_block,
            factories.to_vec(),
            amms.to_vec(),
        );

        let file = File::create(TESTING_CHECKPOINT_PATH)?;
        serde_json::to_writer(file, &cp)?;

        Ok(())
    }

    fn load_testing_checkpoint(&self) -> Result<Checkpoint> {
        let file = File::open(TESTING_CHECKPOINT_PATH)?;
        let cp: Checkpoint = serde_json::from_reader(file)?;
        Ok(cp)
    }

    /// Subscribes to state changes in the provided AMMs starting from the latest synced block.

    async fn subscribe_pools(
        &self,
        provider: Arc<RootProvider<PubSubFrontend>>,
        amms: &[AMM],
        latest_synced_block: u64,
    ) -> Result<Receiver<Vec<Address>>> {
        debug!(
            "Setting up state change subscription from block {}",
            latest_synced_block + 1
        );
        let subscriber = StateSpaceManager::new(amms.to_vec(), provider);

        let (receiver, _) = subscriber
            .subscribe_state_changes(latest_synced_block, 100)
            .await?;

        Ok(receiver)
    }

    /// Synchronizes or loads AMMs from a checkpoint file.

    async fn sync_or_load_amms(
        &self,
        provider: Arc<RootProvider<PubSubFrontend>>,
        factories: Vec<Factory>,
        checkpoint_path: &str,
    ) -> Result<(Vec<AMM>, u64)> {
        // Define factory configurations

        let path = Path::new(checkpoint_path);

        // Either load from checkpoint or sync from chain
        debug!("Checking for checkpoint file at {}", checkpoint_path);
        let (amms, last_synced_block) = if path.exists() {
            info!("Loading AMMs from checkpoint file");
            let (_, amms, last_synced_block) =
                sync_amms_from_checkpoint(checkpoint_path, 1000, provider.clone()).await?;
            (amms, last_synced_block)
        } else {
            info!("No checkpoint found, syncing AMMs from chain");
            sync_amms(factories, provider.clone(), Some(checkpoint_path), 1000).await?
        };

        Ok((amms, last_synced_block))
    }

    /// Filter AMMs based on usdt threshold.
    async fn filter_amms(
        &self,
        amms: Vec<AMM>,
        factories: Vec<Factory>,
        usdt_threshold: f64,
        provider: Arc<RootProvider<PubSubFrontend>>,
    ) -> Result<Vec<AMM>> {
        let usdc = Address::from_str(USDC).unwrap();
        let weth_address = Address::from_str(WETH).unwrap();
        let usd_weth_adrr = Address::from_str(WETH_USDC_PAIR).unwrap();
        let weth_value = U256::from(WETH_VALUE);

        let usd_weth_pool = AMM::UniswapV2Pool(
            UniswapV2Pool::new_from_address(usd_weth_adrr, None, 300, provider.clone()).await?,
        );

        let amms = filter_amms_below_usd_threshold(
            amms,
            &factories,
            usd_weth_pool,
            usdt_threshold, //Setting usd_threshold to 15000 filters out any pool that contains less than $15000.00 USD value
            weth_address,
            usdc,
            weth_value,
            200,
            provider.clone(),
        )
        .await?;

        Ok(amms)
        // 10 weth
    }

    /// Builds the token graph by iterating over all pools and adding nodes and edges.
    ///
    /// This method collects all the pools from the pools map, and then iterates over them in parallel to build the nodes and edges of the token graph. For each pool, it adds nodes for the two tokens, and then calculates the weights for the edges between the tokens based on the pool reserves.
    ///
    /// # Returns
    /// A `Result` indicating whether the token graph was successfully built.
    pub async fn build_token_graph(&self) -> Result<()> {
        let pools_map = self.config.app_state.pools_map.clone();
        let token_graph = self.config.app_state.token_graph.clone();
        let token_graph = token_graph.write().await;

        // Collect all pools
        let pools: Vec<AMM> = pools_map
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        // Iterate over the pools in parallel to build nodes and edges
        pools.par_iter().for_each(|amm| {
            let tokens = amm.tokens();
            let token0 = tokens[0];
            let token1 = tokens[1];

            // Add nodes for tokens if they don't exist
            token_graph.add_node(Node { address: token0 });
            token_graph.add_node(Node { address: token1 });

            // Calculate weights for edges based on pool reserves
            if let Ok(rate0_to_1) = amm.calculate_price(token0, token1) {
                let weight = -rate0_to_1.ln();
                token_graph.upsert_edge(&token0, &token1, weight, amm.address());
            }

            if let Ok(rate1_to_0) = amm.calculate_price(token1, token0) {
                let weight = -rate1_to_0.ln();

                token_graph.upsert_edge(&token1, &token0, weight, amm.address());
            }
        });

        Ok(())
    }
    /// Updates the token graph with modified pools.
    ///
    /// This method updates the existing token graph using the latest reserve data from modified pools. It recalculates
    /// the weights of edges connected to the modified pools, ensuring that the graph remains up-to-date for accurate
    /// arbitrage detection.
    ///
    /// # Arguments
    /// * `updated_pools` - A list of AMMs that have changed and need to be updated in the graph.
    pub async fn update_token_graph(&self, updated_pools: &Vec<AMM>) -> Result<()> {
        let token_graph = self.config.app_state.token_graph.clone();
        let token_graph = token_graph.write().await;

        token_graph.update_by_pools(updated_pools);
        drop(token_graph);
        Ok(())
    }
}
