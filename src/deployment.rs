use crate::events::{decimalsCall, nameCall, symbolCall};
use crate::rpc::RpcClient;
use alloy_primitives::Address;
use anyhow::Result;
use tracing::{info, warn};

pub async fn find_deployment_block(
    client: &RpcClient,
    address: Address,
    latest_block: u64,
) -> Result<u64> {
    info!("Searching for deployment block of contract {:?}", address);

    let code = client.get_code_at_block(address, latest_block).await?;
    if code.is_empty() {
        anyhow::bail!("Address {:?} is not a deployed contract", address);
    }

    let mut left = 0u64;
    let mut right = latest_block;

    while left < right {
        let mid = (left + right) / 2;

        let code = client.get_code_at_block(address, mid).await?;

        if code.is_empty() {
            left = mid + 1;
        } else {
            right = mid;
        }
    }

    info!("Contract deployed at block {}", left);
    Ok(left)
}

#[derive(Debug, Clone)]
pub struct TokenMetadata {
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
}

pub async fn fetch_token_metadata(client: &RpcClient, address: Address) -> Result<TokenMetadata> {
    info!("Fetching token metadata for {:?}", address);

    // Try to fetch name
    let name = match client.call_contract(address, nameCall {}).await {
        Ok(result) => {
            info!("Token name: {}", result);
            Some(result)
        }
        Err(e) => {
            warn!("Failed to fetch token name: {}", e);
            None
        }
    };

    // Try to fetch symbol
    let symbol = match client.call_contract(address, symbolCall {}).await {
        Ok(result) => {
            info!("Token symbol: {}", result);
            Some(result)
        }
        Err(e) => {
            warn!("Failed to fetch token symbol: {}", e);
            None
        }
    };

    // Try to fetch decimals
    let decimals = match client.call_contract(address, decimalsCall {}).await {
        Ok(result) => {
            info!("Token decimals: {}", result);
            Some(result)
        }
        Err(e) => {
            warn!("Failed to fetch token decimals: {}", e);
            None
        }
    };

    Ok(TokenMetadata {
        name,
        symbol,
        decimals,
    })
}
