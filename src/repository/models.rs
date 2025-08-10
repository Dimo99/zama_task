use alloy_primitives::{Address, B256, U256};

#[derive(Debug, Clone)]
pub struct Token {
    pub address: Address,
    pub deployment_block: u64,
    pub last_processed_block: Option<u64>,
    pub last_processed_finalized_block: Option<u64>,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub transaction_hash: B256,
    pub log_index: u64,
    pub token_address: Address,
    pub from_address: Address,
    pub to_address: Address,
    pub value: U256,
    pub block_number: u64,
    pub block_hash: B256,
    pub is_finalized: bool,
}
