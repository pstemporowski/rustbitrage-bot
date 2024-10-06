use alloy_primitives::hex;
use ethers::types::{H256, I256};
use ethers::utils::format_units;
use log::warn;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use ethers::types::{Address, U256};
use std::cmp::Ordering;

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
        let decoded = hex::decode(constants::SYNC_TOPIC).unwrap();
        let sync_topic = H256::from_slice(&decoded);

        loop {
            if let Some(opportunity) = self.config.opportunity_receiver.lock().await.recv().await {
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
                            pending_state_updates.push(PotentialStateUpdate {
                                address,
                                reserve0,
                                reserve1
                            });

                            affected_pairs.push(address);
                        }
                    }

                    if pending_state_updates.is_empty() { continue }

                    let cycles = self.find_optimal_cycles(Some(affected_pairs));

                    let after: Duration = opportunity.time.elapsed();
                    if !cycles.is_empty() {
                        warn!(
                            "                  ------> BackRun Tx Hash {:?}",
                            opportunity.tx.hash()
                        );
                        warn!(
                            "                  ------> Profit: {:.9} ",
                            format_units(cycles[0].profit, "ether").unwrap()
                        );
                        warn!(
                            "                  ------> Optimal WETH In: {:.9} ",
                            format_units(cycles[0].optimal_in, "ether").unwrap()
                        );
                        warn!(
                            "                  ------> E2E time: {:?} ",
                            after
                        );
                        warn!(
                            "             ",
                        );
                    }
                        }
                    }
    }

    pub fn find_optimal_cycles(
            &self,
            affected_pairs: Option<Vec<Address>>,
        ) -> Vec<NetPositiveCycle> {
            let cycles_mapping = self.config.mapping.cycles_mapping.read().unwrap();
            let pairs_mapping = self.config.mapping.pairs_mapping.read().unwrap();

            let mut pointers: Vec<&Vec<IndexedPair>> = Vec::new();

            match affected_pairs {
                Some(affected_pairs) => {
                    affected_pairs.iter().for_each(|pair_address| {
                        if let Some(cycle) = cycles_mapping.get(pair_address) {
                            pointers.extend(cycle.iter());
                        }                
                    });   
                }
                None => {
                    for (_, cycles) in cycles_mapping.iter() {
                        pointers.extend(cycles.iter());
                    }
                }
            }

            let mut net_profit_cycles = Vec::new();

            let weth = Address::from_slice(WETH.as_bytes());

            for cycle in pointers {
                let pairs = cycle
                    .iter()
                    .filter_map(|pair| pairs_mapping.get(&pair.address))
                    .collect::<Vec<&Arc<RwLock<UniV2Pool>>>>();

                let pairs_clone = pairs.clone();
                let profit_function =
                    move |amount_in: U256| -> I256 { self.get_profit(weth, amount_in, &pairs_clone) };

                let optimal = self.maximize_profit(
                    U256::one(),
                    U256::from_dec_str("10000000000000000000000").unwrap(),
                    U256::from_dec_str("10").unwrap(),
                    profit_function,
                );

                let (profit, swap_amounts) = self.get_profit_with_amount(weth, optimal, &pairs);

                let mut cycle_internal = Vec::new();
                for pair in pairs {
                    cycle_internal.push(pair.read().unwrap().address);
                }

                if profit > I256::one() {
                    let net_positive_cycle = NetPositiveCycle {
                        profit,
                        optimal_in: optimal,
                        cycle_addresses: cycle_internal,
                        swap_amounts,
                    };
                    net_profit_cycles.push(net_positive_cycle);
                }
            }

            net_profit_cycles.sort();
            net_profit_cycles.into_iter().take(5).collect()
        }

    fn maximize_profit(
        &self,
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

        (domain_max + domain_min) / 2
    }

    pub fn get_profit(&self, token_in: Address, amount_in: U256, pairs: &Vec<&Arc<RwLock<UniV2Pool>>>) -> I256 {
        let mut amount_out: U256 = amount_in;
        let mut token_in = token_in;
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
            token_in = if pair.token0 == token_in {
                pair.token1
            } else {
                pair.token0
            };
        }

        I256::from_raw(amount_out) - I256::from_raw(amount_in)
    }

    pub fn get_profit_with_amount(
        &self,
        token_in: Address,
        amount_in: U256,
        pairs: &Vec<&Arc<RwLock<UniV2Pool>>>,
    ) -> (I256, Vec<U256>) {
        let mut amount_out: U256 = amount_in;
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

        (
            I256::from_raw(amount_out) - I256::from_raw(amount_in),
            amounts,
        )
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