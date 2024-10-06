use log::{debug, info};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use ethers::types::{Address, H160};


use crate::types::uni_v2_pool::UniV2Pool;
use crate::utils::constants::WETH;
use crate::utils::helpers::address;


/// Points to the addresses, this makes state updates easier
#[derive(Debug, Clone, Copy)]
pub struct IndexedPair {
    pub address: usize,

    pub token0: usize,
    pub token1: usize,
}


pub type Cycle = Vec<IndexedPair>;


pub struct Mapping {
    /// For indexed pointer to address
    pub index_mapping: Arc<RwLock<HashMap<usize, Address>>>,
    /// For address to indexed pointer
    pub address_mapping: Arc<RwLock<HashMap<Address, usize>>>,
    /// Pointer to the pool
    pub pairs_mapping: Arc<RwLock<HashMap<usize, Arc<RwLock<UniV2Pool>>>>>,
    /// For easy access at pending state
    pub cycles_mapping: Arc<RwLock<HashMap<Address, Vec<Cycle>>>>,

}


impl Mapping {

    pub fn new(pools: &Vec<UniV2Pool>) -> Self {
        let mut address_mapping = HashMap::new();
        let mut index_mapping = HashMap::new();
        let mut pairs_mapping = HashMap::new();

        for pair in pools.iter() {

            let current_len = index_mapping.len();
            index_mapping.insert(current_len, pair.address);
            address_mapping.insert(pair.address, current_len);

            let token0_exists = address_mapping.contains_key(&pair.token0);
            if !token0_exists {
                let current_len = index_mapping.len();
                index_mapping.insert(current_len, pair.token0);
                address_mapping.insert(pair.token0, current_len);
            }

            let token1_exists = address_mapping.contains_key(&pair.token1);
            if !token1_exists {
                let current_len = index_mapping.len();
                index_mapping.insert(current_len, pair.token1);
                address_mapping.insert(pair.token1, current_len);
            }
        }

        let mut indexed_pairs = Vec::new();
        for pair in pools.iter() {
            let indexed_pair = IndexedPair {
                address: *address_mapping.get(&pair.address).unwrap(),
                token0: *address_mapping.get(&pair.token0).unwrap(),
                token1: *address_mapping.get(&pair.token1).unwrap(),
            };

            indexed_pairs.push(indexed_pair);
            pairs_mapping.insert(
                *address_mapping.get(&pair.address).unwrap(),
                Arc::new(RwLock::new(pair.clone())),
            );
        }

        let weth_index = *address_mapping.get(&address(WETH)).unwrap();
        let now = std::time::Instant::now();

        let cycles = Self::find_cycles(
            &indexed_pairs,
            weth_index,
            weth_index,
            3, // should be enough
            &Vec::new(),
            &mut Vec::new(),
            &mut HashSet::new(),
        );

        info!("Number of cycles: {:?}", cycles.len());
        info!("Time took for finding all cycles: {:?}", now.elapsed());

        let mut cycles_mapping: HashMap<H160, Vec<Vec<IndexedPair>>> = HashMap::new();

        for indexed_cycle in cycles.iter() {
            for indexed_pair in indexed_cycle {
                cycles_mapping
                    .entry(index_mapping[&indexed_pair.address])
                    .or_insert_with(Vec::new)
                    .push(indexed_cycle.clone());
            }
        }

        Self {
            index_mapping: Arc::new(RwLock::new(index_mapping)),
            address_mapping: Arc::new(RwLock::new(address_mapping)),
            pairs_mapping: Arc::new(RwLock::new(pairs_mapping)),
            cycles_mapping: Arc::new(RwLock::new(cycles_mapping)),
        }
      }

      /// Find cycles using DFS
    fn find_cycles(
        pairs: &[IndexedPair],
        token_in: usize,
        token_out: usize,
        max_hops: i32,
        current_pairs: &Vec<IndexedPair>,
        circles: &mut Vec<Cycle>,
        seen: &mut HashSet<usize>,
    ) -> Vec<Cycle> {
        let mut circles_copy: Vec<Cycle> = circles.clone();

        for pair in pairs {
            if seen.contains(&pair.address) {
                continue;
            }

            let temp_out: usize;
            if token_in == pair.token0 {
                temp_out = pair.token1;
            } else if token_in == pair.token1 {
                temp_out = pair.token0;
            } else {
                continue;
            }

            let mut new_seen = seen.clone();
            new_seen.insert(pair.address);

            if temp_out == token_out {
                let mut new_cycle = current_pairs.clone();
                new_cycle.push(*pair);
                circles_copy.push(new_cycle);
            } else if max_hops > 1 {
                let mut new_pairs: Vec<IndexedPair> = current_pairs.clone();
                new_pairs.push(*pair);
                circles_copy = Self::find_cycles(
                    pairs,
                    temp_out,
                    token_out,
                    max_hops - 1,
                    &new_pairs,
                    &mut circles_copy,
                    &mut new_seen,
                );
            }
        }

        circles_copy
    }
    
}