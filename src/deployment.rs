use crate::rpc::RpcClient;
use alloy_primitives::Address;
use anyhow::Result;
use tracing::info;

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
