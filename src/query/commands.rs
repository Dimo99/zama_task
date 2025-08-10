use crate::query::formatters::{
    OutputFormat, format_balance, format_stats, format_top_holders, format_transfers,
};
use crate::repository::{TokenRepository, TransferRepository};
use alloy_primitives::Address;
use anyhow::Result;
use std::str::FromStr;

pub fn cmd_balance(
    transfer_repo: &TransferRepository,
    token_repo: &TokenRepository,
    token_address: &Address,
    address: &str,
    format: &OutputFormat,
) -> Result<()> {
    let address = Address::from_str(address)
        .map_err(|_| anyhow::anyhow!("Invalid address format: {}", address))?;

    let balance_info = transfer_repo.get_balance(&address)?;
    let decimals = token_repo.get_token_decimals(token_address)?;
    let output = format_balance(balance_info, decimals, format);
    println!("{output}");

    Ok(())
}

#[derive(Default)]
pub struct TransferQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub block: Option<u64>,
    pub block_range: Option<(u64, u64)>,
    pub limit: usize,
    pub offset: usize,
}

pub fn cmd_transfers(
    transfer_repo: &TransferRepository,
    token_repo: &TokenRepository,
    token_address: &Address,
    query: TransferQuery,
    format: &OutputFormat,
) -> Result<()> {
    // Parse addresses if provided
    let from_address = query
        .from
        .as_ref()
        .map(|addr| {
            Address::from_str(addr).map_err(|_| anyhow::anyhow!("Invalid from address: {}", addr))
        })
        .transpose()?;

    let to_address = query
        .to
        .as_ref()
        .map(|addr| {
            Address::from_str(addr).map_err(|_| anyhow::anyhow!("Invalid to address: {}", addr))
        })
        .transpose()?;

    // Handle block or block_range
    let block_range = if let Some(block_num) = query.block {
        Some((block_num, block_num))
    } else {
        query.block_range
    };

    // Check if at least one filter is provided
    if from_address.is_none() && to_address.is_none() && block_range.is_none() {
        return Err(anyhow::anyhow!(
            "Please specify at least one filter: --from, --to, --block, or --block-range"
        ));
    }

    let transfers = transfer_repo.query_transfers(
        from_address.as_ref(),
        to_address.as_ref(),
        block_range,
        query.limit,
        query.offset,
    )?;

    let decimals = token_repo.get_token_decimals(token_address)?;
    let output = format_transfers(&transfers, decimals, format);
    println!("{output}");

    Ok(())
}

pub fn cmd_top_holders(
    transfer_repo: &TransferRepository,
    token_repo: &TokenRepository,
    token_address: &Address,
    count: usize,
    format: &OutputFormat,
) -> Result<()> {
    let holders = transfer_repo.get_top_holders(count)?;
    let decimals = token_repo.get_token_decimals(token_address)?;
    let output = format_top_holders(holders, decimals, format);
    println!("{output}");

    Ok(())
}

pub fn cmd_stats(repo: &TransferRepository, format: &OutputFormat) -> Result<()> {
    let stats = repo.get_statistics()?;
    let output = format_stats(&stats, format);
    println!("{output}");

    Ok(())
}

pub fn cmd_address_history(
    transfer_repo: &TransferRepository,
    token_repo: &TokenRepository,
    token_address: &Address,
    address: &str,
    limit: usize,
    offset: usize,
    format: &OutputFormat,
) -> Result<()> {
    let address = Address::from_str(address)
        .map_err(|_| anyhow::anyhow!("Invalid address format: {}", address))?;

    let transfers = transfer_repo.get_address_history(&address, limit, offset)?;
    let decimals = token_repo.get_token_decimals(token_address)?;
    let output = format_transfers(&transfers, decimals, format);
    println!("{output}");

    Ok(())
}
