# Rustbitrage Bot

A high-performance arbitrage bot written in Rust that monitors DEX pools for profitable trading opportunities. Stopped on development due to lack of time and the profitability of arbitrage opportunities.

## Current Features

- Real-time mempool monitoring for pending transactions
- Uniswap V2 pool integration and state tracking
- Negative cycle detection using Bellman-Ford algorithm
- Concurrent transaction processing with semaphore-based rate limiting
- Efficient graph-based token relationship tracking
- Checkpoint system for fast pool state recovery
- USD value-based pool filtering

## Missing Features

- Support for other DEX protocols (currently only Uniswap V2)
- Gas optimization for backrunning transactions
- Multi-hop arbitrage execution
- Slippage protection mechanisms
- Position sizing and risk management
- Exution of arbitrage trades

## Known Issues

There is currently a deadlock issue in the mempool state updates that affects the Bellman-Ford calculation performance. This occurs when:

1. The mempool watcher attempts to update pool states
2. Concurrent access to the shared token graph causes lock
3. This leads to slower negative cycle detection than optimal

The deadlock particularly impacts the arbitrage opportunity detection speed when there are many pool state changes happening simultaneously.

## Next Steps

Priority fixes needed:
1. Implement fine-grained locking for graph updates
2. Add lock-free data structures where possible
3. Optimize the concurrency model for mempool updates