/// Module for watching mempool and managing AMM state changes
use std::{path::Path, sync::Arc};

use alloy::{
    primitives::{address, Address},
    providers::RootProvider,
    pubsub::PubSubFrontend,
};
use amms::{
    amm::{factory::Factory, uniswap_v2::factory::UniswapV2Factory, AutomatedMarketMaker, AMM},
    state_space::StateSpaceManager,
    sync::{checkpoint::sync_amms_from_checkpoint, sync_amms},
};
use dashmap::DashMap;
use eyre::Result;
use log::{debug, info};
use petgraph::graph::Graph;
use rayon::prelude::*;
use tokio::{
    sync::{mpsc::Receiver, Mutex},
    time::Instant,
};

use crate::modules::config::Config;

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
    ///
    /// This function performs the following steps:
    /// 1. Synchronizes or loads AMMs from a checkpoint file.
    /// 2. Initializes the pool map with the synchronized AMMs.
    /// 3. Builds the initial token graph.
    /// 4. Sets up a subscription for state changes.
    /// 5. Enters a main loop to process state changes, updating the token graph as necessary.
    ///
    /// The function returns a `Result` indicating whether the service ran successfully.
    pub async fn run(&self) -> Result<()> {
        info!("Starting MempoolWatcher service");
        let now = Instant::now();
        let checkpoint_path = ".cfmms-checkpoint.json";

        let provider = self.config.provider.clone();
        let pools_map = self.config.app_state.pools_map.clone();

        // Synchronize or load AMMs from checkpoint
        debug!("Starting AMM synchronization process");
        let (amms, latest_synced_block) = self
            .sync_or_load_amms(provider.clone(), checkpoint_path)
            .await?;

        // Initialize pool map with synchronized AMMs
        debug!("Initializing pool map with {} AMMs", amms.len());
        amms.par_iter().for_each(|amm| {
            pools_map.insert(amm.address(), amm.clone());
        });

        info!(
            "Pool initialization completed: {} pools in {} ms",
            pools_map.len(),
            now.elapsed().as_millis()
        );

        // Build initial token graph
        debug!("Starting token graph construction");
        self.build_token_graph().await?;
        info!("Token graph construction completed");

        // Set up subscription for state changes
        debug!("Setting up state change subscription");
        let mut state_change_receiver = self
            .subscribe_pools(provider.clone(), &amms, latest_synced_block)
            .await?;

        // Main loop for processing state changes
        info!("MempoolWatcher is now monitoring for state changes");
        while let Some(changed_addresses) = state_change_receiver.recv().await {
            debug!("Processing {} state changes", changed_addresses.len());
            let updated_pools: Vec<AMM> = changed_addresses
                .into_iter()
                .filter_map(|addr| {
                    if let Some(entry) = pools_map.get(&addr) {
                        Some(entry.value().clone())
                    } else {
                        None
                    }
                })
                .collect();

            if !updated_pools.is_empty() {
                debug!(
                    "Updating token graph with {} modified pools",
                    updated_pools.len()
                );
                self.update_token_graph(&updated_pools).await?;
            }
        }

        Ok(())
    }

    /// Builds the initial token graph for the application.
    ///
    /// This function initializes the token graph data structures, collects all unique tokens from the registered AMMs, adds the tokens as nodes to the graph, and calculates and adds the edge weights based on the exchange rates between the tokens.
    ///
    /// The resulting token graph and token indices are stored in the application state for later use.
    async fn build_token_graph(&self) -> Result<()> {
        debug!("Initializing token graph data structures");
        let graph = Arc::new(Mutex::new(Graph::<Address, f64>::new()));
        let indices_map = Arc::new(DashMap::new());
        let tokens_map: DashMap<Address, ()> = DashMap::new();
        let pools_map = self.config.app_state.pools_map.clone();

        // Collect unique tokens from all pools
        debug!("Collecting unique tokens from pools");
        pools_map.par_iter().for_each(|entry| {
            let amm = entry.value();
            let tokens = amm.tokens();
            let token0 = tokens[0];
            let token1 = tokens[1];

            tokens_map.insert(token0, ());
            tokens_map.insert(token1, ());
        });

        // Add tokens as graph nodes
        debug!("Adding {} tokens as graph nodes", tokens_map.len());
        for entry in tokens_map.iter() {
            let token = *entry.key();
            let mut graph = graph.lock().await;
            let index = graph.add_node(token);
            indices_map.insert(token, index);
        }

        // Calculate and add edge weights based on exchange rates
        debug!("Calculating and adding edge weights");
        for entry in pools_map.iter() {
            let amm = entry.value();
            let tokens = amm.tokens();
            let token0 = tokens[0];
            let token1 = tokens[1];

            let index0 = indices_map.get(&token0).unwrap();
            let index1 = indices_map.get(&token1).unwrap();

            let amount_in = 1_000_000_000_000_000_000u128;

            // Calculate and add edges for both directions
            if let Ok(amount_out) = amm.calculate_price(token0, token1) {
                let rate = amount_out as f64 / amount_in as f64;
                let weight = -rate.ln();
                let mut graph = graph.lock().await;
                graph.update_edge(*index0, *index1, weight);
            }

            if let Ok(amount_out) = amm.calculate_price(token1, token0) {
                let rate = amount_out as f64 / amount_in as f64;
                let weight = -rate.ln();
                let mut graph = graph.lock().await;
                graph.update_edge(*index1, *index0, weight);
            }
        }

        // Update application state with new graph
        debug!("Finalizing token graph initialization");
        let token_graph = self.config.app_state.token_graph.clone();
        let token_indices = self.config.app_state.token_indices.clone();

        let mut token_graph = token_graph.write().await;
        *token_graph = Arc::try_unwrap(graph).unwrap().into_inner();

        let mut token_indices = token_indices.write().await;
        *token_indices = Arc::try_unwrap(indices_map).unwrap();

        let mut is_initialized = self.config.app_state.is_initialized.write().await;
        *is_initialized = true;

        Ok(())
    }

    /// Updates the token graph with the provided updated AMMs.
    ///
    /// This function takes a slice of updated AMMs and updates the token graph and token indices accordingly. It adds any new tokens to the graph, and updates the edge weights based on the new exchange rates.
    ///
    /// # Arguments
    /// * `updated_pools` - A slice of updated AMM instances.
    ///
    /// # Returns
    /// A `Result` indicating whether the update was successful.
    async fn update_token_graph(&self, updated_pools: &[AMM]) -> Result<()> {
        debug!("Updating token graph with {} pools", updated_pools.len());
        let token_graph = self.config.app_state.token_graph.clone();
        let token_indices = self.config.app_state.token_indices.clone();
        let mut token_graph = token_graph.write().await;
        let token_indices = token_indices.write().await;

        for amm in updated_pools {
            let tokens = amm.tokens();
            let token0 = tokens[0];
            let token1 = tokens[1];

            // Add new tokens to graph if they don't exist
            let index0 = match token_indices.get(&token0) {
                Some(index) => *index,
                None => {
                    debug!("Adding new token to graph: {:?}", token0);
                    let idx = token_graph.add_node(token0);
                    token_indices.insert(token0, idx);
                    idx
                }
            };
            let index1 = match token_indices.get(&token1) {
                Some(index) => *index,
                None => {
                    debug!("Adding new token to graph: {:?}", token1);
                    let idx = token_graph.add_node(token1);
                    token_indices.insert(token1, idx);
                    idx
                }
            };

            // Update edge weights based on new exchange rates
            if let Ok(rate) = amm.calculate_price(token0, token1) {
                let weight = -rate.ln();
                token_graph.update_edge(index0, index1, weight);
            }

            if let Ok(rate) = amm.calculate_price(token1, token0) {
                let weight = -rate.ln();
                token_graph.update_edge(index1, index0, weight);
            }
        }

        Ok(())
    }

    /// Subscribes to state changes in the provided AMMs starting from the latest synced block.
    ///
    /// This function creates a new `StateSpaceManager` instance with the provided AMMs and provider, and then
    /// subscribes to state changes starting from the latest synced block plus one. It returns a `Receiver`
    /// that can be used to receive the updated addresses of the AMMs.
    ///
    /// # Arguments
    /// * `provider` - An `Arc<RootProvider<PubSubFrontend>>` instance used for interacting with the blockchain.
    /// * `amms` - A slice of `AMM` instances to subscribe to.
    /// * `latest_synced_block` - The latest block number that has been synced.
    ///
    /// # Returns
    /// A `Receiver` that can be used to receive the updated addresses of the AMMs.
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
            .subscribe_state_changes(latest_synced_block + 1, 100)
            .await?;

        Ok(receiver)
    }

    /// Synchronizes or loads AMMs from a checkpoint file.
    ///
    /// This function checks for the existence of a checkpoint file at the provided path. If the file exists, it loads the AMMs and the last synced block number from the checkpoint. If the file does not exist, it synchronizes the AMMs from the chain using the provided factory configurations.
    ///
    /// # Arguments
    /// * `provider` - An `Arc<RootProvider<PubSubFrontend>>` instance used for interacting with the blockchain.
    /// * `checkpoint_path` - The path to the checkpoint file.
    ///
    /// # Returns
    /// A tuple containing the vector of AMMs and the last synced block number.
    async fn sync_or_load_amms(
        &self,
        provider: Arc<RootProvider<PubSubFrontend>>,
        checkpoint_path: &str,
    ) -> Result<(Vec<AMM>, u64)> {
        let now = Instant::now();
        // Define factory configurations
        let factories = vec![
            Factory::UniswapV2Factory(UniswapV2Factory::new(
                address!("5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f"),
                10207858,
                300,
            )),
            Factory::UniswapV2Factory(UniswapV2Factory::new(
                address!("C0AEe478e3658e2610c5F7A4A2E1777cE9e4f2Ac"),
                10794229,
                300,
            )),
        ];
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

        info!(
            "AMM synchronization completed: {} AMMs in {} ms",
            amms.len(),
            now.elapsed().as_millis()
        );

        Ok((amms, last_synced_block))
    }
}
