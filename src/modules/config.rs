use std::fs::File;
use std::str::FromStr;
use std::sync::Arc;
use ethers::types::{BlockNumber, U256};
use log::{debug, info};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, mpsc};
use ethers::providers::{Provider, Ws};
use ethers::prelude::Wallet;
use ethers::prelude::k256::ecdsa::SigningKey;
use crate::types::opportunity::Opportunity;
use crate::types::uni_v2_pool::UniV2Pool;

use super::pool_mapper::Mapping;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Storage {
    pub pools: Vec<UniV2Pool>,
    pub block: U256,
}

pub struct AppState {
    pub base_fee: Arc<RwLock<U256>>,
    pub current_block_number: Arc<RwLock<BlockNumber>>,
}

pub struct TransactionRequest {
}

pub struct Config {
    pub wss: Arc<Provider<Ws>>,
    pub app_state: Arc<AppState>,
    pub base_fee: Arc<RwLock<U256>>,
    pub wallet: Arc<Wallet<SigningKey>>,
    pub execute_tx_sender: mpsc::Sender<TransactionRequest>,
    pub execute_tx_receiver: Mutex<mpsc::Receiver<TransactionRequest>>,
    pub opportunity_sender: mpsc::Sender<Opportunity>,
    pub opportunity_receiver: Mutex<mpsc::Receiver<Opportunity>>,
    pub mapping: Mapping,
}

impl Config {
    pub async fn new(ws_url: String, priv_key: String) -> Self {
        let (execute_tx_sender, execute_tx_receiver) = mpsc::channel(100);
        let (pending_tx_sender, pending_tx_receiver) = mpsc::channel(100);
        
        let wss = Arc::new(Provider::<Ws>::connect(&ws_url).await.unwrap());
        let base_fee = Arc::new(RwLock::new(U256::from(0)));
        let wallet = Arc::new(Wallet::from_str(&priv_key).unwrap());
        let app_state = Arc::new(AppState {
            current_block_number: Arc::new(RwLock::new(BlockNumber::Latest)),
            base_fee: base_fee.clone(),
        });

        debug!("Loading pools...");
        let pools = Self::load_pools("/Users/pstemporowski/dev/rusty/data/db.json").unwrap();
        let mapping = Mapping::new(&pools);
        
        info!("Loaded {} pools", pools.len());
        Self {
            wss,
            wallet,
            base_fee,
            app_state,
            execute_tx_sender,
            opportunity_sender: pending_tx_sender,
            mapping,
            execute_tx_receiver: Mutex::new(execute_tx_receiver),
            opportunity_receiver: Mutex::new(pending_tx_receiver),
        }
    }

    fn load_pools(path: &str) -> Result<Vec<UniV2Pool>, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let storage: Storage = serde_json::from_reader(reader)?;
        Ok(storage.pools)
    }
}