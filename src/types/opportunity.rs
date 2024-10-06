use std::time::Instant;

use ethers::types::{CallLogFrame, Transaction};


pub struct Opportunity {
    pub tx: Transaction,
    pub logs: Vec<CallLogFrame>,
    pub time: Instant,
}
