use alloy::primitives::Address;

pub fn get_token_pair(tokens: Vec<Address>) -> (Address, Address) {
    let mut token0 = tokens[0];
    let mut token1 = tokens[1];

    if token1 > token0 {
        let temp = token0;
        token0 = token1;
        token1 = temp;
    }

    (token0, token1)
}
