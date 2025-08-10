use anyhow::Result;
use eth_indexer::config::Config;
use eth_indexer::repository::Database;
use eth_indexer::rpc::RpcClient;
use eth_indexer::scanner::Scanner;
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

    let client = RpcClient::new(&config.json_rpc_urls, &config)?;
    info!("RPC client connected");

    let mut scanner = Scanner::new(client, db, &config)?;

    if let Err(e) = scanner.run().await {
        error!("Scanner error: {}", e);
        return Err(e);
    }

    Ok(())
}
