use alloy::{
    primitives::{Address, U256},
    providers::RootProvider,
    pubsub::PubSubFrontend,
    rpc::types::{trace::geth::CallLogFrame, Transaction},
    sol_types::SolEvent,
};
use amms::amm::AMM;
use dashmap::DashMap;
use eyre::Result;
use log::{info, warn};
use std::sync::Arc;

use crate::types::swap_event::Swap;

use super::call_frame::get_call_frame;

#[derive(Debug, Clone)]
pub struct SwapInfo {
    pub from_token: Address,
    pub to_token: Address,
    pub pool_address: Address,
    pub version: u8,
    pub amount_in: U256,
}

pub async fn extract_swaps(
    provider: Arc<RootProvider<PubSubFrontend>>,
    tx: &Transaction,
    pools_map: &DashMap<Address, AMM>,
) -> Result<Vec<SwapInfo>> {
    let mut swaps = Vec::new();

    let frame = get_call_frame(&tx, provider).await?;

    for log in frame.logs {
        if let Some(swap_info) = process_log(log, pools_map)? {
            swaps.push(swap_info);
        }
    }

    Ok(swaps)
}

pub fn process_log(
    log: CallLogFrame,
    pools_map: &DashMap<Address, AMM>,
) -> Result<Option<SwapInfo>> {
    // Ensure we have topics and data
    if let (Some(topics), Some(data), Some(pool_address)) = (log.topics, log.data, log.address) {
        // Check if the first topic matches the Swap event signature
        if topics.len() <= 1 {
            return Ok(None);
        }

        if !Swap::SIGNATURE_HASH.eq(&topics[0]) {
            // This is not a Swap event
            warn!("Topic does not match Swap event signature");
            return Ok(None);
        } else {
            warn!("Swap event detected");
        }

        // Fetch the pool from the map
        let pool = match pools_map.get(&pool_address) {
            Some(pool) => pool.clone(),
            None => {
                warn!("Pool {} not found in the map", pool_address);
                return Ok(None);
            }
        };

        if let AMM::UniswapV2Pool(pool) = pool {
            // Decode the Swap event data
            let decoded_swap = Swap::abi_decode_data(&data, true)?;
            let (amount0_in, amount1_in, amount0_out, amount1_out) = decoded_swap;
            let token_0 = pool.token_a;
            let token_1 = pool.token_b;

            info!("Amount0 in: {}", amount0_in);
            info!("Amount1 in: {}", amount1_in);
            info!("Amount0 out: {}", amount0_out);
            info!("Amount1 out: {}", amount1_out);

            if amount0_in > U256::ZERO {
                let swap_info = SwapInfo {
                    from_token: token_0,
                    to_token: token_1,
                    pool_address,
                    version: 2,
                    amount_in: amount0_in,
                };
                return Ok(Some(swap_info));
            } else if amount1_in > U256::ZERO {
                let swap_info = SwapInfo {
                    from_token: token_1,
                    to_token: token_0,
                    pool_address,
                    version: 2,
                    amount_in: amount1_in,
                };
                return Ok(Some(swap_info));
            } else {
                // This is not a swap event
                warn!("Unsupported");
                return Ok(None);
            }
        } else {
            // Handle other pool types if necessary
            // TODO : Add other pool types
            warn!("Unsupported pool type");
            return Ok(None);
        }
    } else {
        warn!("No topics or data");
    }

    Ok(None)
}
