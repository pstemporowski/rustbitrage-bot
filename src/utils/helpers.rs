use ethers::prelude::*;

/// Converts &str to Address.
pub fn address(address: &str) -> Address {
    address.parse::<Address>().unwrap()
}