use alloy::{
    primitives::{Address, Uint},
    rpc::types::{trace::geth::CallFrame, Transaction},
    sol,
    sol_types::SolCall,
};
use amms::amm::AMM;
use dashmap::DashMap;
use eyre::Result;
use log::warn;

sol!(
    #[allow(missing_docs)]
    function swapExactTokensForTokens(
        uint256 amountIn,
        uint256 amountOutMin,
        address[] calldata path,
        address to,
        uint256 deadline
    ) external returns (uint256[] memory amounts);
    function swapTokensForExactTokens(
        uint256 amountOut,
        uint256 amountInMax,
        address[] calldata path,
        address to,
        uint256 deadline
    ) external returns (uint256[] memory amounts);

    function swapExactETHForTokens(
        uint256 amountOutMin,
        address[] calldata path,
        address to,
        uint256 deadline
    ) external payable returns (uint256[] memory amounts);

    function swapTokensForExactETH(
        uint256 amountOut,
        uint256 amountInMax,
        address[] calldata path,
        address to,
        uint256 deadline
    ) external returns (uint256[] memory amounts);

    function swapExactTokensForETH(
        uint256 amountIn,
        uint256 amountOutMin,
        address[] calldata path,
        address to,
        uint256 deadline
    ) external returns (uint256[] memory amounts);

    function swapETHForExactTokens(
        uint256 amountOut,
        address[] calldata path,
        address to,
        uint256 deadline
    ) external payable returns (uint256[] memory amounts);
    function swap(
        uint256 amount0Out,
        uint256 amount1Out,
        address to,
        bytes calldata data
    ) external;
);

#[derive(Debug, Clone)]
pub struct SwapInfo {
    pub path: Vec<Address>,
    pub amount_in: Uint<256, 4>,
}

pub fn decode_swap_function(tx: &Transaction) -> Result<Option<SwapInfo>> {
    let input_data = tx.input.clone();
    if let Ok(decoded) = swapExactTokensForTokensCall::abi_decode(&input_data, false) {
        Ok(Some(SwapInfo {
            path: decoded.path,
            amount_in: decoded.amountIn,
        }))
    } else if let Ok(decoded) = swapTokensForExactTokensCall::abi_decode(&input_data, true) {
        Ok(Some(SwapInfo {
            path: decoded.path,
            amount_in: decoded.amountInMax,
        }))
    } else if let Ok(decoded) = swapExactETHForTokensCall::abi_decode(&input_data, true) {
        Ok(Some(SwapInfo {
            path: decoded.path,
            amount_in: tx.value,
        }))
    } else if let Ok(decoded) = swapTokensForExactETHCall::abi_decode(&input_data, true) {
        Ok(Some(SwapInfo {
            path: decoded.path,
            amount_in: decoded.amountInMax,
        }))
    } else if let Ok(decoded) = swapExactTokensForETHCall::abi_decode(&input_data, true) {
        Ok(Some(SwapInfo {
            path: decoded.path,
            amount_in: decoded.amountIn,
        }))
    } else if let Ok(decoded) = swapETHForExactTokensCall::abi_decode(&input_data, true) {
        Ok(Some(SwapInfo {
            path: decoded.path,
            amount_in: tx.value,
        }))
    } else {
        warn!("Unknown or unsupported swap function in input data.");
        Ok(None)
    }
}

/// Recursively checks the call frame and its subcalls to find any interactions with known AMM pools.
///
/// This function takes a `CallFrame` object, a map of pool addresses to AMM instances, and a mutable
/// vector of AMM instances. It checks if the call's target address is a known pool address, and if
/// so, adds the corresponding AMM instance to the `interactions` vector. It then recursively
/// checks any subcalls in the `CallFrame` object.
pub fn get_pool_interactions<'a>(
    call_frame: &'a CallFrame,
    pools_map: &'a DashMap<Address, AMM>,
    interactions: &mut Vec<AMM>,
) {
    if let Some(to_address) = call_frame.to {
        if let Some(pool) = pools_map.get(&to_address) {
            interactions.push(pool.value().clone());
        }
    }

    for subcall in &call_frame.calls {
        get_pool_interactions(subcall, pools_map, interactions);
    }
}
