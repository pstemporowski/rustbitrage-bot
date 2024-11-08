use std::sync::Arc;

use alloy::{
    primitives::Address,
    providers::{Provider, RootProvider},
    pubsub::PubSubFrontend,
};
use amms::amm::AMM;
use dashmap::DashMap;
use log::{debug, info, warn};
use tokio::sync::RwLock;

use crate::{modules::graph::Graph, types::pending_tx_hash::PendingTx, utils::swap::extract_swaps};

/// Processes a single pending transaction, focusing on Uniswap V2 interactions and related DeFi operations.
///
/// # Arguments
/// * `tx_hash` - The hash of the pending transaction to process.
/// * `provider` - The Ethereum provider to use for fetching transaction data.
/// * `config` - The application configuration, including the current block number and base fee.
///
/// # Returns
/// * `eyre::Result<()>` - Result indicating success or failure of the processing.
pub async fn process_transaction(
    pending_tx: PendingTx,
    provider: Arc<RootProvider<PubSubFrontend>>,
    block_number: u64,
    next_base_fee: u64,
    pools_map: &DashMap<Address, AMM>,
    token_graph: Arc<RwLock<Graph>>,
) -> eyre::Result<()> {
    let provider = provider.clone();
    let received_at = pending_tx.received_at;
    let hash = pending_tx.hash;
    // let received_at = pending_tx.received_at;

    // Early exit if block number or base fee is not set.
    if block_number == 0 || next_base_fee == 0 {
        debug!(
            "Block number or base fee not set. Skipping transaction {}",
            hash
        );
        return Ok(());
    }

    // Early filtering: Skip transactions with low gas price.
    // Fetching minimal transaction info to check gas price.
    if let Some(tx) = provider.get_transaction_by_hash(hash).await? {
        // If transaction does have a blocknumber, skip it as it's already mined
        if tx.block_number.is_some() {
            debug!("Transaction {} is already mined. Skipping.", hash);
            return Ok(());
        }

        if tx.max_fee_per_gas.unwrap_or(0) < u128::from(next_base_fee) {
            debug!(
                "Transaction {} has insufficient max fee per gas. Skipping.",
                hash
            );
            return Ok(());
        }

        let swaps = match extract_swaps(provider, &tx, &pools_map).await {
            Ok(swaps) => swaps,
            Err(_) => {
                debug!(
                    "Failed to extract swaps from transaction {}. Skipping.",
                    hash
                );
                return Ok(());
            }
        };

        if swaps.len() > 0 {
            info!("Found {} swaps in transaction {}", swaps.len(), hash);
        }

        // Simulate swaps and check for arbitrage opportunities
        let opportunities = {
            let graph = token_graph.read().await;
            graph
                .find_opportunities(&swaps, &pools_map)
                .unwrap_or_else(|e| {
                    warn!(
                        "Failed to find arbitrage opportunities after simulation: {}",
                        e
                    );
                    Vec::new()
                })
        };

        opportunities.iter().for_each(|op| {
            info!(
                "\n🔥 ARBITRAGE OPPORTUNITY FOUND! 🔥\n\
                \n📍 Path: {:?}\
                \n💰 Expected Profit: {} WEI\
                \n🔗 Backrun Transaction: {}\
                \n💎 Optimal Amount In: {}\
                \n----------------------------------",
                op.path, op.expected_profit, tx.hash, op.optimal_amount_in,
            );
        });

        let time_since_received = received_at.elapsed();

        info!(
            "Processed transaction: {} in {} ms",
            tx.hash,
            time_since_received.as_millis()
        );
    } else {
        debug!(
            "Transaction {} not found. It might have been dropped.",
            hash
        );
    }

    Ok(())
}
