use alloy::primitives::{Address, U256};

/// Represents an arbitrage opportunity with the optimal amount and expected profit.
pub struct ArbitrageOpportunity {
    pub path: Vec<Address>,
    pub optimal_amount_in: U256,
    pub expected_profit: U256,
}
