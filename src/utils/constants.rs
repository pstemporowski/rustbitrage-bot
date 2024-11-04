//ETH Mainnet adresses
pub const WETH: &str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
pub const USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
pub const WETH_USDC_PAIR: &str = "0xb4e16d0168e52d35cacd2c6185b44281ec28c9dc";

// Thresholds for AMM filtering
pub const MIN_USD_FACTORY_THRESHOLD: f64 = 100000.0;
pub const WETH_VALUE: u128 = 100000000000000000_u128;

// Paths for checkpoint files
pub const TESTING_CHECKPOINT_PATH: &str = ".dev-cfmms-checkpoint.json";

pub const SWAP_FUNCTION_SIGNATURES: &[&[u8]] = &[
    // Uniswap V2 pair swap function signature
    &[0x02, 0x2c, 0x0d, 0x9f],
    // Add other swap function selectors if needed
];

// Add at the end
pub const MAX_ITERATIONS: usize = 10;
