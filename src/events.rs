use alloy::rpc::types::Log;
use alloy::sol;
use alloy::sol_types::SolEvent;

sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
}

pub fn decode_transfer_event(log: &Log) -> anyhow::Result<Transfer> {
    let log_data = log.data();
    let decoded = Transfer::decode_raw_log(log.topics(), &log_data.data)?;
    Ok(decoded)
}