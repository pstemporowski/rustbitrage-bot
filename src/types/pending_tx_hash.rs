use alloy::primitives::FixedBytes;
use tokio::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTx {
    pub hash: FixedBytes<32>,
    pub received_at: Instant,
}

impl PendingTx {
    pub fn new(hash: FixedBytes<32>) -> Self {
        Self {
            hash,
            received_at: Instant::now(),
        }
    }
}
