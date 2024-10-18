use alloy_primitives::hex;
use ethers::types::{H256, I256};
use ethers::utils::format_units;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::error::TrySendError;
use tokio::time::Instant;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use rayon::prelude::*;
use std::collections::HashMap;


use ethers::types::{Address, U256};
use std::cmp::Ordering;

use crate::types::opportunity::ArbitrageTransaction;
use crate::utils::constants;
use crate::{types::uni_v2_pool::UniV2Pool, utils::constants::WETH};
use super::{config::Config, pool_mapper::IndexedPair};

#[derive(Debug, Deserialize)]
pub struct NetPositiveCycle {
    pub profit: I256,
    pub optimal_in: U256,
    pub swap_amounts: Vec<U256>,
    pub cycle_addresses: Vec<Address>,
}

impl Ord for NetPositiveCycle {
    fn cmp(&self, other: &Self) -> Ordering {
        other.profit.cmp(&self.profit)
    }
}

impl PartialOrd for NetPositiveCycle {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for NetPositiveCycle {}

impl PartialEq for NetPositiveCycle {
    fn eq(&self, other: &Self) -> bool {
        self.profit == other.profit
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct PotentialStateUpdate {
    pub address: Address,
    pub reserve0: U256,
    pub reserve1: U256,
}
pub struct OpportunityFinder {
    config: Arc<Config>,
}

impl OpportunityFinder {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run(&mut self) {
        info!("Starting opportunity finder");
        let now = Instant::now();
        let decoded = hex::decode(constants::SYNC_TOPIC).unwrap();
        let sync_topic = H256::from_slice(&decoded);

        // At initialization
        let reserves_cache = Arc::new(RwLock::new(HashMap::new()));

        loop {
            if let Some(opportunity) = self.config.potential_opportunity_receiver.lock().await.recv().await {
                info!("Found potential opportunity: {:?}", opportunity.tx);
                let mut pending_state_updates = Vec::new();
                let mut affected_pairs = Vec::new();

                for log in opportunity.logs {
                    let topics = match log.topics {
                        Some(d) => d,
                        None => continue
                    };

                      let data = match log.data {
                        Some(d) => d,
                        None => continue
                    };

                    let address = match log.address {
                        Some(d) => d,
                        None => continue
                    };
                    debug!("Processing log: {:?}", log.address);
                      let mut reserve0 = U256::zero();
                        let mut reserve1: U256 = U256::zero();
                        let mut found_swap = false;

                        for topic in topics {
                            if topic == sync_topic {
                                reserve0 = U256::from_big_endian(&data[0..32]);
                                reserve1 = U256::from_big_endian(&data[32..]);
                                found_swap = true;
                            }
                        }
                    
                        if found_swap {    
                            debug!("Found swap: {:?}", log.address);
                            pending_state_updates.push(PotentialStateUpdate {
                                address,
                                reserve0,
                                reserve1
                            });

                            affected_pairs.push(address);
                            debug!("Pushed state update: {:?}", log.address);
                        }
                    }

                    if pending_state_updates.is_empty() { continue }

                    debug!("Pending state updates: {:?}", pending_state_updates);

                    // Update reserves_cache when there's a state change
                    for update in &pending_state_updates {
                        let pair_index = self.config.mapping.address_mapping.read().unwrap()[&update.address];
                        reserves_cache.write().unwrap().insert(
                            pair_index,
                            (update.reserve0, update.reserve1),
                        );
                    }

                    // Pass reserves_cache to find_optimal_cycles
                    let cycles = self.find_optimal_cycles(Some(affected_pairs), reserves_cache.clone());
                    now.elapsed();

                    
                    debug!("Time took to find optimal cycles: {:?}", now.elapsed());
                    let after: Duration = opportunity.time.elapsed();
                    if !cycles.is_empty() {
                        let best_cycle = &cycles[0];

                        warn!(
                            "                  ------> BackRun Tx Hash {:?}",
                            opportunity.tx.hash()
                        );
                        warn!(
                            "                  ------> Profit: {:.9} ",
                            format_units(best_cycle.profit, "ether").unwrap()
                        );
                        warn!(
                            "                  ------> Optimal WETH In: {:.9} ",
                            format_units(best_cycle.optimal_in, "ether").unwrap()
                        );
                        warn!(
                            "                  ------> E2E time: {:?} ",
                            after
                        );
                        warn!("             ");

                        // Create a TransactionRequest
                        let tx_request = ArbitrageTransaction {
                            amount_in: best_cycle.optimal_in,
                            cycle_addresses: best_cycle.cycle_addresses.clone(),
                            profit: best_cycle.profit,
                        };

                        // Send it through execute_tx_sender
                        match self.config.execute_tx_sender.try_send(tx_request) {
                            Ok(_) => (),
                            Err(TrySendError::Full(_)) => continue,
                            Err(TrySendError::Closed(_)) => break,
                        }
                    }
                }
            }
    }
    pub fn find_optimal_cycles(
            &self,
            affected_pairs: Option<Vec<Address>>,
            reserves_cache: Arc<RwLock<HashMap<usize, (U256, U256)>>>,
        ) -> Vec<NetPositiveCycle> {

        let cycles_mapping = self.config.mapping.cycles_mapping.read().unwrap();
        let pairs_mapping = self.config.mapping.pairs_mapping.read().unwrap();
        let mut pointers: Vec<&Vec<IndexedPair>> = Vec::new();
        
        debug!("Starting to find optimal cycles. Affected pairs: {:?}", affected_pairs);
        match affected_pairs {
            Some(ref affected_pairs) => {
                for pair_address in affected_pairs {
                    if let Some(cycle) = cycles_mapping.get(pair_address) {
                        pointers.extend(cycle.iter());
                    }
                }
            }
            None => {
                for cycles in cycles_mapping.values() {
                    pointers.extend(cycles.iter());
                }
            }
        }

        
        let weth = Address::from_str(WETH).expect("Invalid WETH address");

        let mut net_profit_cycles: Vec<_> = pointers.par_iter().filter_map(|cycle| {
            // Move the profit calculation inside the closure
            let pairs = cycle
                .iter()
                .filter_map(|pair| pairs_mapping.get(&pair.address))
                .collect::<Vec<_>>();

            let profit_function = |amount_in: U256| -> I256 {
                self.get_profit_with_reserves(weth, amount_in, &pairs, &reserves_cache)
            };

            let optimal_in = self.maximize_profit(
                U256::one(),
                U256::from_dec_str("10000000000000000000000").unwrap(),
                U256::from_dec_str("10").unwrap(),
                profit_function,
            );

            let (profit, swap_amounts) = self.get_profit_with_amount(weth, optimal_in, &pairs);
            debug!("Cycle profit: {}, optimal_in: {}", profit, optimal_in);

            if profit > I256::zero() {
                let cycle_addresses = pairs.iter().map(|pair| pair.read().unwrap().address).collect();
                Some(NetPositiveCycle {
                    profit,
                    optimal_in,
                    swap_amounts,
                    cycle_addresses,
                })
            } else {
                None
            }

        }).collect();



        info!("Found {} profitable cycles. Best profit: {}", net_profit_cycles.len(), net_profit_cycles[0].profit);
        net_profit_cycles.sort();
        net_profit_cycles.into_iter().take(5).collect()
    }    
        
        
    fn maximize_profit(        &self,
        mut domain_min: U256,
        mut domain_max: U256,
        lowest_delta: U256,
        f: impl Fn(U256) -> I256,
    ) -> U256 {
        loop {
            if domain_max > domain_min {
                if (domain_max - domain_min) > lowest_delta {
                    let mid = (domain_min + domain_max) / 2;

                    let lower_mid = (mid + domain_min) / 2;
                    let upper_mid = (mid + domain_max) / 2;

                    let f_output_lower = f(lower_mid);
                    let f_output_upper = f(upper_mid);

                    if f_output_lower > f_output_upper {
                        domain_max = mid;
                    } else {
                        domain_min = mid;
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        debug!("Maximizing profit. Current domain: [{}, {}]", domain_min, domain_max);

        (domain_max + domain_min) / 2
    }


pub fn get_profit_with_reserves(
    &self,
    token_in: Address,
    amount_in: U256,
    pairs: &[&Arc<RwLock<UniV2Pool>>],
    reserves_cache: &Arc<RwLock<HashMap<usize, (U256, U256)>>>,
) -> I256 {
    let mut amount_out = amount_in;
    let mut token_in = token_in;

    for pair_lock in pairs.iter() {
        let pair = pair_lock.read().unwrap();
        let pair_index = self.config.mapping.address_mapping.read().unwrap()[&pair.address];
        let (reserve_in, reserve_out) = reserves_cache.read().unwrap()[&pair_index];

        let (reserve_in, reserve_out, fees) = if pair.token0 == token_in {
            (reserve_in, reserve_out, pair.fees1)
        } else {
            (reserve_out, reserve_in, pair.fees0)
        };

        let fee_multiplier = pair.router_fee.saturating_sub(fees);
        amount_out = amount_out
            .saturating_mul(fee_multiplier)
            .saturating_mul(reserve_out)
            / (
                reserve_in
                    .saturating_mul(U256::from(10000))
                    .saturating_add(amount_out.saturating_mul(pair.router_fee))
            );
        token_in = if pair.token0 == token_in { pair.token1 } else { pair.token0 };
    }

    // Subtract flash loan fee
    let flash_loan_fee = amount_in * U256::from(9) / U256::from(10000);

    I256::from_raw(amount_out).saturating_sub(I256::from_raw(amount_in + flash_loan_fee))
}

pub fn get_profit_with_amount(
    &self,
    token_in: Address,
    amount_in: U256,
    pairs: &Vec<&Arc<RwLock<UniV2Pool>>>,
) -> (I256, Vec<U256>) {
    let mut amount_out = amount_in;
    let mut token_in = token_in;
    let mut amounts = Vec::with_capacity(pairs.len() + 1);
    amounts.push(amount_in);

    for pair in pairs {
        let pair = pair.read().unwrap();
        let fees;
        let (reserve0, reserve1) = if pair.token0 == token_in {
            fees = pair.fees1;
            (pair.reserve0, pair.reserve1)
        } else {
            fees = pair.fees0;
            (pair.reserve1, pair.reserve0)
        };
        amount_out = self.get_amount_out(amount_out, reserve0, reserve1, fees, pair.router_fee);
        amounts.push(amount_out);
        token_in = if pair.token0 == token_in {
            pair.token1
        } else {
            pair.token0
        };
    }

    // Calculate flash loan fee (0.09% of the loan amount)
    let flash_loan_fee = amount_in * U256::from(9) / U256::from(10000);

    // Total profit is amount_out - amount_in - flash_loan_fee
    let profit = I256::from_raw(amount_out)
        .saturating_sub(I256::from_raw(amount_in))
        .saturating_sub(I256::from_raw(flash_loan_fee));

    (profit, amounts)
}
    pub fn get_amount_out(
        &self,
        a_in: U256,
        reserve_in: U256,
        reserve_out: U256,
        fees: U256,
        router_fee: U256,
    ) -> U256 {
        if a_in == U256::zero() {
            return U256::zero();
        }
        let a_in_with_fee = a_in.saturating_mul(router_fee);
        let a_out = a_in_with_fee.saturating_mul(reserve_out)
            / U256::from(10000)
                .saturating_mul(reserve_in)
                .saturating_add(a_in_with_fee);

        a_out - a_out.saturating_mul(fees) / U256::from(10000)
    }
}
