use std::sync::Arc;

use alloy::{
    primitives::{Address, Bytes},
    rpc::types::trace::geth::CallFrame,
};
use dashmap::DashSet;

pub fn is_uniswap_v2_interaction(
    call_frame: &CallFrame,
    uniswap_v2_addresses: &Arc<DashSet<Address>>,
) -> bool {
    // Check if the call's target is in the set of Uniswap V2 addresses
    if let Some(to_address) = call_frame.to {
        if uniswap_v2_addresses.contains(&to_address) {
            return true;
        }
    }

    // Recursively check subcalls
    for subcall in &call_frame.calls {
        if is_uniswap_v2_interaction(subcall, uniswap_v2_addresses) {
            return true;
        }
    }

    false
}

pub fn extract_token0(input: Bytes) -> Option<Address> {
    // UniswapV2 function signatures for methods that interact with tokens
    const SWAP_EXACT_TOKENS: &[u8] = &[0x38, 0xed, 0x17, 0x39]; // swapExactTokensForTokens
    const SWAP_TOKENS_EXACT: &[u8] = &[0x8e, 0x3f, 0x7b, 0x88]; // swapTokensForExactTokens

    if input.len() < 68 {
        return None;
    }

    // Check function signature (first 4 bytes)
    let sig = &input[0..4];
    if sig == SWAP_EXACT_TOKENS || sig == SWAP_TOKENS_EXACT {
        // Token path starts at offset 4 + 64 + 64 = 132 (after function sig and uint256 params)
        if input.len() >= 164 {
            // 132 + 32 (for path length)
            // Extract first token address from path
            let token = Address::from_slice(&input[132..152]);
            return Some(token);
        }
    }

    None
}

pub fn extract_token1(input: Bytes) -> Option<Address> {
    const SWAP_EXACT_TOKENS: &[u8] = &[0x38, 0xed, 0x17, 0x39];
    const SWAP_TOKENS_EXACT: &[u8] = &[0x8e, 0x3f, 0x7b, 0x88];

    if input.len() < 68 {
        return None;
    }

    let sig = &input[0..4];
    if sig == SWAP_EXACT_TOKENS || sig == SWAP_TOKENS_EXACT {
        // Path length is at offset 132
        if input.len() >= 164 {
            let path_length = u32::from_be_bytes([input[132], input[133], input[134], input[135]]);

            // Last token in path is at offset 132 + (path_length - 1) * 20
            let last_token_offset = 132 + ((path_length as usize - 1) * 20);
            if input.len() >= last_token_offset + 20 {
                let token = Address::from_slice(&input[last_token_offset..last_token_offset + 20]);
                return Some(token);
            }
        }
    }

    None
}
