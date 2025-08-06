use alloy_primitives::Address;

#[derive(Debug, Clone)]
pub struct Token {
    pub address: Address,
    pub deployment_block: u64,
    pub last_processed_block: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub transaction_hash: String,
    pub log_index: u64,
    pub token_address: Address,
    pub from_address: Address,
    pub to_address: Address,
    pub value: String,
    pub block_number: u64,
}