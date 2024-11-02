use std::sync::Arc;

use alloy::{primitives::Address, rpc::types::trace::geth::CallFrame};
use amms::amm::AMM;
use dashmap::DashMap;

/// Checks if the given call frame represents an interaction with a pool managed by the provided AMM map.
///
/// This function recursively checks the call frame and any subcalls to determine if the call is interacting with a pool.
/// If the call's target address is found in the provided AMM map, the function returns `true`. Otherwise, it recursively
/// checks any subcalls and returns `true` if any of them interact with a pool.
pub fn is_pool_interaction(call_frame: &CallFrame, pools_map: &Arc<DashMap<Address, AMM>>) -> bool {
    // Check if the call's target is in the set of Uniswap V2 addresses
    if let Some(to_address) = call_frame.to {
        if pools_map.contains_key(&to_address) {
            return true;
        }
    }

    // Recursively check subcalls
    for subcall in &call_frame.calls {
        if is_pool_interaction(subcall, pools_map) {
            return true;
        }
    }

    false
}
