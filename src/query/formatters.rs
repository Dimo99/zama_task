use crate::repository::{BalanceInfo, TokenHolder, Transfer, TransferStats};
use alloy_primitives::utils::format_units;
use comfy_table::{Cell, Table, modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL};
use csv::Writer;
use serde_json::json;

#[derive(Debug, Clone)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}

impl From<&str> for OutputFormat {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" => OutputFormat::Json,
            "csv" => OutputFormat::Csv,
            _ => OutputFormat::Table,
        }
    }
}

pub fn format_transfers(
    transfers: &[Transfer],
    decimals: Option<u8>,
    format: &OutputFormat,
) -> String {
    match format {
        OutputFormat::Table => format_transfers_table(transfers, decimals),
        OutputFormat::Json => format_transfers_json(transfers, decimals),
        OutputFormat::Csv => format_transfers_csv(transfers, decimals),
    }
}

fn format_transfers_table(transfers: &[Transfer], decimals: Option<u8>) -> String {
    if transfers.is_empty() {
        return "No transfers found.".to_string();
    }

    let decimals = decimals.unwrap_or(18);
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_header(vec![
            "Block",
            "From",
            "To",
            "Value",
            "Value (Wei)",
            "Tx Hash",
        ]);

    for transfer in transfers {
        let formatted_value =
            format_units(transfer.value, decimals).unwrap_or_else(|_| transfer.value.to_string());
        table.add_row(vec![
            Cell::new(transfer.block_number),
            Cell::new(format!("{:#}", transfer.from_address)),
            Cell::new(format!("{:#}", transfer.to_address)),
            Cell::new(formatted_value),
            Cell::new(transfer.value.to_string()),
            Cell::new(format_tx_hash(&format!("{:?}", transfer.transaction_hash))),
        ]);
    }

    table.to_string()
}

fn format_transfers_json(transfers: &[Transfer], decimals: Option<u8>) -> String {
    let decimals = decimals.unwrap_or(18);
    let json_transfers: Vec<_> = transfers
        .iter()
        .map(|t| {
            let formatted_value =
                format_units(t.value, decimals).unwrap_or_else(|_| t.value.to_string());
            json!({
                "block_number": t.block_number,
                "transaction_hash": format!("{:?}", t.transaction_hash),
                "log_index": t.log_index,
                "from": format!("{:?}", t.from_address),
                "to": format!("{:?}", t.to_address),
                "value": formatted_value,
                "value_wei": t.value.to_string(),
            })
        })
        .collect();

    serde_json::to_string_pretty(&json_transfers).unwrap_or_else(|_| "[]".to_string())
}

fn format_transfers_csv(transfers: &[Transfer], decimals: Option<u8>) -> String {
    let decimals = decimals.unwrap_or(18);
    let mut wtr = Writer::from_writer(vec![]);

    // Write header
    let _ = wtr.write_record([
        "block_number",
        "from",
        "to",
        "value",
        "value_wei",
        "transaction_hash",
        "log_index",
    ]);

    // Write records
    for transfer in transfers {
        let formatted_value =
            format_units(transfer.value, decimals).unwrap_or_else(|_| transfer.value.to_string());
        let _ = wtr.write_record([
            &transfer.block_number.to_string(),
            &format!("{:?}", transfer.from_address),
            &format!("{:?}", transfer.to_address),
            &formatted_value,
            &transfer.value.to_string(),
            &format!("{:?}", transfer.transaction_hash),
            &transfer.log_index.to_string(),
        ]);
    }

    String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
}

pub fn format_balance(
    balance_info: BalanceInfo,
    decimals: Option<u8>,
    format: &OutputFormat,
) -> String {
    let decimals = decimals.unwrap_or(18); // Default to 18 decimals for most ERC20 tokens
    let balance_formatted = format_units(balance_info.balance, decimals)
        .unwrap_or_else(|_| balance_info.balance.to_string());
    let incoming_formatted = format_units(balance_info.total_incoming, decimals)
        .unwrap_or_else(|_| balance_info.total_incoming.to_string());
    let outgoing_formatted = format_units(balance_info.total_outgoing, decimals)
        .unwrap_or_else(|_| balance_info.total_outgoing.to_string());

    match format {
        OutputFormat::Table => {
            let mut table = Table::new();
            table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_header(vec!["Metric", "Value (Formatted)", "Value (Wei)"]);

            table.add_row(vec![
                Cell::new("Balance"),
                Cell::new(&balance_formatted),
                Cell::new(balance_info.balance.to_string()),
            ]);
            table.add_row(vec![
                Cell::new("Total Incoming"),
                Cell::new(&incoming_formatted),
                Cell::new(balance_info.total_incoming.to_string()),
            ]);
            table.add_row(vec![
                Cell::new("Total Outgoing"),
                Cell::new(&outgoing_formatted),
                Cell::new(balance_info.total_outgoing.to_string()),
            ]);
            table.to_string()
        }
        OutputFormat::Json => json!({
            "balance": balance_formatted,
            "balance_wei": balance_info.balance.to_string(),
            "total_incoming": incoming_formatted,
            "total_incoming_wei": balance_info.total_incoming.to_string(),
            "total_outgoing": outgoing_formatted,
            "total_outgoing_wei": balance_info.total_outgoing.to_string(),
        })
        .to_string(),
        OutputFormat::Csv => {
            let mut wtr = Writer::from_writer(vec![]);
            let _ = wtr.write_record(["metric", "value_formatted", "value_wei"]);
            let _ = wtr.write_record([
                "balance",
                &balance_formatted,
                &balance_info.balance.to_string(),
            ]);
            let _ = wtr.write_record([
                "total_incoming",
                &incoming_formatted,
                &balance_info.total_incoming.to_string(),
            ]);
            let _ = wtr.write_record([
                "total_outgoing",
                &outgoing_formatted,
                &balance_info.total_outgoing.to_string(),
            ]);
            String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
        }
    }
}

pub fn format_top_holders(
    holders: Vec<TokenHolder>,
    decimals: Option<u8>,
    format: &OutputFormat,
) -> String {
    match format {
        OutputFormat::Table => format_top_holders_table(&holders, decimals),
        OutputFormat::Json => format_top_holders_json(&holders, decimals),
        OutputFormat::Csv => format_top_holders_csv(&holders, decimals),
    }
}

fn format_top_holders_table(holders: &[TokenHolder], decimals: Option<u8>) -> String {
    if holders.is_empty() {
        return "No holders found.".to_string();
    }

    let decimals = decimals.unwrap_or(18);
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_header(vec!["Rank", "Address", "Balance", "Balance (Wei)"]);

    for (i, holder) in holders.iter().enumerate() {
        let formatted_balance =
            format_units(holder.balance, decimals).unwrap_or_else(|_| holder.balance.to_string());
        table.add_row(vec![
            Cell::new(i + 1),
            Cell::new(format!("{:#}", &holder.address)),
            Cell::new(formatted_balance),
            Cell::new(holder.balance.to_string()),
        ]);
    }

    table.to_string()
}

fn format_top_holders_json(holders: &[TokenHolder], decimals: Option<u8>) -> String {
    let decimals = decimals.unwrap_or(18);
    let json_holders: Vec<_> = holders
        .iter()
        .enumerate()
        .map(|(i, holder)| {
            let formatted = format_units(holder.balance, decimals)
                .unwrap_or_else(|_| holder.balance.to_string());
            json!({
                "rank": i + 1,
                "address": holder.address,
                "balance": formatted,
                "balance_wei": holder.balance.to_string(),
            })
        })
        .collect();

    serde_json::to_string_pretty(&json_holders).unwrap_or_else(|_| "[]".to_string())
}

fn format_top_holders_csv(holders: &[TokenHolder], decimals: Option<u8>) -> String {
    let decimals = decimals.unwrap_or(18);
    let mut wtr = Writer::from_writer(vec![]);

    let _ = wtr.write_record(["rank", "address", "balance", "balance_wei"]);

    for (i, holder) in holders.iter().enumerate() {
        let formatted =
            format_units(holder.balance, decimals).unwrap_or_else(|_| holder.balance.to_string());
        let _ = wtr.write_record([
            &(i + 1).to_string(),
            &format!("{:?}", holder.address),
            &formatted,
            &holder.balance.to_string(),
        ]);
    }

    String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
}

pub fn format_stats(stats: &TransferStats, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Table => {
            let mut table = Table::new();
            table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_header(vec!["Metric", "Value"]);

            table.add_row(vec![
                Cell::new("Total Transfers"),
                Cell::new(stats.total_transfers),
            ]);
            table.add_row(vec![
                Cell::new("Unique Addresses"),
                Cell::new(stats.unique_addresses),
            ]);
            table.add_row(vec![
                Cell::new("Earliest Block"),
                Cell::new(
                    stats
                        .earliest_block
                        .map_or("N/A".to_string(), |b| b.to_string()),
                ),
            ]);
            table.add_row(vec![
                Cell::new("Latest Block"),
                Cell::new(
                    stats
                        .latest_block
                        .map_or("N/A".to_string(), |b| b.to_string()),
                ),
            ]);

            table.to_string()
        }
        OutputFormat::Json => serde_json::to_string_pretty(&json!({
            "total_transfers": stats.total_transfers,
            "unique_addresses": stats.unique_addresses,
            "earliest_block": stats.earliest_block,
            "latest_block": stats.latest_block,
        }))
        .unwrap_or_else(|_| "{}".to_string()),
        OutputFormat::Csv => {
            let mut wtr = Writer::from_writer(vec![]);
            let _ = wtr.write_record(["metric", "value"]);
            let _ = wtr.write_record(["total_transfers", &stats.total_transfers.to_string()]);
            let _ = wtr.write_record(["unique_addresses", &stats.unique_addresses.to_string()]);
            let _ = wtr.write_record([
                "earliest_block",
                &stats
                    .earliest_block
                    .map_or("N/A".to_string(), |b| b.to_string()),
            ]);
            let _ = wtr.write_record([
                "latest_block",
                &stats
                    .latest_block
                    .map_or("N/A".to_string(), |b| b.to_string()),
            ]);
            String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
        }
    }
}

fn format_tx_hash(hash: &str) -> String {
    format!("{}...{}", &hash[..6], &hash[hash.len() - 4..])
}
