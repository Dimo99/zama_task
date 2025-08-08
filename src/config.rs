use alloy_primitives::Address;
use anyhow::{Context, Result};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Config {
    pub json_rpc_urls: Vec<String>,
    pub erc20_contract_address: Address,
    pub database_url: String,
    pub batch_size: u64,
    pub rate_limit_delay_ms: u64,
    pub max_pending_requests: usize,
    pub request_timeout_secs: u64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();

        let json_rpc_urls = if let Ok(urls) = std::env::var("JSON_RPC_URLS") {
            urls.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else if let Ok(url) = std::env::var("JSON_RPC_URL") {
            vec![url]
        } else {
            return Err(anyhow::anyhow!(
                "Either JSON_RPC_URLS or JSON_RPC_URL must be set in .env"
            ));
        };

        if json_rpc_urls.is_empty() {
            return Err(anyhow::anyhow!("At least one RPC URL must be provided"));
        }

        let contract_address_str = std::env::var("ERC20_CONTRACT_ADDRESS")
            .context("ERC20_CONTRACT_ADDRESS must be set in .env")?;

        let erc20_contract_address = Address::from_str(&contract_address_str)
            .context("Invalid ERC20_CONTRACT_ADDRESS format")?;

        let database_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:./indexer.db".to_string());

        Ok(Config {
            json_rpc_urls,
            erc20_contract_address,
            database_url,
            batch_size: std::env::var("BATCH_SIZE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1000),
            rate_limit_delay_ms: std::env::var("RATE_LIMIT_DELAY_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(500),
            max_pending_requests: std::env::var("MAX_PENDING_REQUESTS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
            request_timeout_secs: std::env::var("REQUEST_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(120),
        })
    }
}
