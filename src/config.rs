use alloy_primitives::Address;
use anyhow::{Context, Result};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Config {
    pub json_rpc_url: String,
    pub erc20_contract_address: Address,
    pub database_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();

        let json_rpc_url = std::env::var("JSON_RPC_URL")
            .context("JSON_RPC_URL must be set in .env")?;

        let contract_address_str = std::env::var("ERC20_CONTRACT_ADDRESS")
            .context("ERC20_CONTRACT_ADDRESS must be set in .env")?;

        let erc20_contract_address = Address::from_str(&contract_address_str)
            .context("Invalid ERC20_CONTRACT_ADDRESS format")?;

        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite:./indexer.db".to_string());

        Ok(Config {
            json_rpc_url,
            erc20_contract_address,
            database_url,
        })
    }
}