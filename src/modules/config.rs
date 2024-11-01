use alloy::primitives::{Address, BlockNumber};
use alloy::providers::{ProviderBuilder, RootProvider, WsConnect};
use alloy::pubsub::PubSubFrontend;
use eyre::Result;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AppState {
    pub next_block_base_fee: Arc<RwLock<u64>>,
    pub block_number: Arc<RwLock<u64>>,
}

pub struct Config {
    pub provider: Arc<RootProvider<PubSubFrontend>>,
    pub app_state: Arc<AppState>,
    pub contract_address: Address,
}
impl Config {
    pub async fn new(ws_url: String) -> Result<Self> {
        let ws = WsConnect::new(&ws_url);
        let provider = Arc::new(ProviderBuilder::new().on_ws(ws).await?);
        let base_fee = Arc::new(RwLock::new(0));
        let app_state: Arc<AppState> = Arc::new(AppState {
            next_block_base_fee: base_fee.clone(),
            block_number: Arc::new(RwLock::new(BlockNumber::MIN)),
        });

        Ok(Self {
            provider,
            app_state,
            contract_address: Address::from_str("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D")?,
        })
    }
}
