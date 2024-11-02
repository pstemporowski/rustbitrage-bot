// Required external crates and dependencies
use alloy::primitives::{Address, BlockNumber};
use alloy::providers::{ProviderBuilder, RootProvider, WsConnect};
use alloy::pubsub::PubSubFrontend;
use amms::amm::AMM;
use dashmap::DashMap;
use eyre::Result;
use petgraph::graph::NodeIndex;
use petgraph::Graph;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Represents the application's shared state across different components
pub struct AppState {
    /// Next block's base fee for gas price estimation
    pub next_block_base_fee: Arc<RwLock<u64>>,
    /// Current block number being processed
    pub block_number: Arc<RwLock<u64>>,
    /// Thread-safe map of AMM pools indexed by their addresses
    pub pools_map: Arc<DashMap<Address, AMM>>,
    /// Graph representation of token relationships with weights
    pub token_graph: Arc<RwLock<Graph<Address, f64>>>,
    /// Mapping of token addresses to their corresponding graph indices
    pub token_indices: Arc<RwLock<DashMap<Address, NodeIndex>>>,
    /// Flag indicating whether the application has completed initialization
    pub is_initialized: Arc<RwLock<bool>>,
}

/// Configuration structure holding core application components
pub struct Config {
    /// WebSocket provider for blockchain interaction
    pub provider: Arc<RootProvider<PubSubFrontend>>,
    /// Shared application state
    pub app_state: Arc<AppState>,
    /// Target contract address for interactions
    pub contract_address: Address,
}

impl Config {
    /// Creates a new Config instance with the provided WebSocket URL
    ///
    /// # Arguments
    /// * `ws_url` - WebSocket URL for connecting to the blockchain node
    ///
    /// # Returns
    /// * `Result<Self>` - New Config instance wrapped in a Result
    pub async fn new(ws_url: String) -> Result<Self> {
        // Initialize WebSocket connection
        let ws = WsConnect::new(&ws_url);
        let provider = Arc::new(ProviderBuilder::new().on_ws(ws).await?);

        // Initialize base fee with default value
        let base_fee = Arc::new(RwLock::new(0));

        // Create new AppState with default values
        let app_state: Arc<AppState> = Arc::new(AppState {
            next_block_base_fee: base_fee.clone(),
            block_number: Arc::new(RwLock::new(BlockNumber::MIN)),
            pools_map: Arc::new(DashMap::new()),
            token_graph: Arc::new(RwLock::new(Graph::new())),
            token_indices: Arc::new(RwLock::new(DashMap::new())),
            is_initialized: Arc::new(RwLock::new(false)),
        });

        Ok(Self {
            provider,
            app_state,
            // Uniswap V2 Router contract address
            contract_address: Address::from_str("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D")?,
        })
    }
}
