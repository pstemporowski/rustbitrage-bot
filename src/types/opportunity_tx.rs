use std::time::Instant;

use ethers::types::{CallLogFrame, Transaction};


pub struct OpportunityTx {
    pub tx: Transaction,
    pub logs: Vec<CallLogFrame>,
    pub time: Instant,
}
