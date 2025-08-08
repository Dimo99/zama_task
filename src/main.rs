mod config;
mod deployment;
mod events;
mod insertion_worker;
mod repository;
mod rpc;
mod scanner;

use anyhow::Result;
use config::Config;
use repository::Database;
use rpc::RpcClient;
use scanner::Scanner;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    info!("Starting Ethereum Log Indexer");

    let config = Config::from_env()?;
    info!("Configuration loaded");
    info!("Contract address: {:?}", config.erc20_contract_address);
    info!(
        "RPC URLs: {} endpoint(s) configured",
        config.json_rpc_urls.len()
    );

    let db = Database::new(&config.database_url)?;
    info!("Database initialized");

    let client = RpcClient::new(&config.json_rpc_urls)?;
    info!("RPC client connected");

    let mut scanner = Scanner::new(client, db, config.erc20_contract_address)?;

    if let Err(e) = scanner.run().await {
        error!("Scanner error: {}", e);
        return Err(e);
    }

    Ok(())
}
