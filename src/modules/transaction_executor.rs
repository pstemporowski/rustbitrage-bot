use abi::{Function, Param, ParamType};
use ethers::prelude::*;
use ethers::providers::Middleware;
use ethers::signers::Signer;
use ethers::types::transaction::eip2718::TypedTransaction;
use ethers::types::TransactionRequest;
use log::info;
use std::sync::Arc;

use crate::modules::config::Config;

pub struct TransactionExecutor {
    config: Arc<Config>,
}

impl TransactionExecutor {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run(&self) {
        let mut receiver = self.config.execute_tx_receiver.lock().await;
        while let Some(tx_request) = receiver.recv().await {
            // Prepare transaction data
            let amount_in = tx_request.amount_in;
            let cycle_addresses = tx_request.cycle_addresses;

            info!("Executing arbitrage transaction. Profit: {}, Amount in: {}", tx_request.profit, tx_request.amount_in);



            // // Build calldata for the smart contract
            // let calldata = self.build_calldata(amount_in, cycle_addresses);

            // // Create transaction request
            // let tx = self.create_transaction(calldata).await;

            // // Send transaction
            // match self.send_transaction(tx).await {
            //     Ok(tx_hash) => info!("Transaction sent: {:?}", tx_hash),
            //     Err(e) => error!("Failed to send transaction: {:?}", e),
            // }

        }
    }

    // fn build_calldata(&self, amount_in: U256, cycle_addresses: Vec<Address>) -> Bytes {
    //     // ABI encode the function call to the smart contract
    //     // Using ethers-rs ABI encoding
    //     use ethers::abi::Token;

    //     let function = Function {
    //         name: "executeArbitrage".to_string(),
    //         inputs: vec![
    //             Param {
    //                 name: "amount".to_string(),
    //                 kind: ParamType::Uint(256),
    //                 internal_type: None,
    //             },
    //             Param {
    //                 name: "path".to_string(),
    //                 kind: ParamType::Array(Box::new(ParamType::Address)),
    //                 internal_type: None,
    //             },
    //         ],
    //         outputs: vec![],
    //         constant: None,
    //         state_mutability: ethers::abi::StateMutability::NonPayable,
    //     };

    //     let tokens = vec![
    //         Token::Uint(amount_in),
    //         Token::Array(
    //             cycle_addresses
    //                 .into_iter()
    //                 .map(Token::Address)
    //                 .collect::<Vec<_>>(),
    //         ),
    //     ];

    //     function.encode_input(&tokens).expect("Failed to encode calldata").into()
    // }

    // async fn create_transaction(&self, calldata: Bytes) -> TypedTransaction {
    //     let contract_address = self.config.contract_address;

    //     let mut tx = TransactionRequest::new()
    //         .to(contract_address)
    //         .data(calldata)
    //         .into();

    //     // Set gas price and gas limit
    //     let gas_price = self.config.wss.get_gas_price().await.expect("Failed to get gas price");
    //     tx.set_gas_price(gas_price);

    //     let gas_limit = U256::from(600_000); // Adjust based on your contract's expected gas usage
    //     tx.set_gas(gas_limit);

    //     tx
    // }

    // async fn send_transaction(&self, tx: TypedTransaction) -> Result<H256, Box<dyn std::error::Error>> {
    //     let provider = self.config.wss.clone();
    //     let chain_id = provider.get_chainid().await?;
    //     let wallet = self.config.wallet.clone().with_chain_id(chain_id.as_u64());
    //     let client = SignerMiddleware::new(provider, wallet);
    //     let pending_tx = client.send_transaction(tx, None).await?;
    //     let tx_hash = pending_tx.tx_hash();
    //     Ok(tx_hash)
    // }
}