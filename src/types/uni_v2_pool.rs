
use ethers::types::{Address, U256};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniV2Pool {
    pub address: Address,

    pub token0: Address,
    pub token1: Address,

    pub reserve0: U256,
    pub reserve1: U256,

    // router fee
    pub router_fee: U256,
    //  token tax when token0 is in
    pub fees0: U256,
    //  token tax when token1 is in
    pub fees1: U256,
}
