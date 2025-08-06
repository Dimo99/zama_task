use alloy::providers::fillers::FillProvider;
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::{BlockNumberOrTag, Filter, Log};
use alloy_primitives::{Address, B256, Bytes};
use anyhow::{Context, Result};

type AlloyFullProvider = FillProvider<
    alloy::providers::fillers::JoinFill<
        alloy::providers::Identity,
        alloy::providers::fillers::JoinFill<
            alloy::providers::fillers::GasFiller,
            alloy::providers::fillers::JoinFill<
                alloy::providers::fillers::BlobGasFiller,
                alloy::providers::fillers::JoinFill<
                    alloy::providers::fillers::NonceFiller,
                    alloy::providers::fillers::ChainIdFiller,
                >,
            >,
        >,
    >,
    alloy::providers::RootProvider,
>;

pub struct RpcClient {
    provider: AlloyFullProvider,
}

impl RpcClient {
    pub fn new(rpc_url: &str) -> Result<Self> {
        let url = rpc_url.parse().context("Invalid RPC URL")?;
        let provider: AlloyFullProvider = ProviderBuilder::new()
            .on_http(url);

        Ok(RpcClient { provider })
    }

    pub async fn get_latest_block(&self) -> Result<u64> {
        let block_number = self
            .provider
            .get_block_number()
            .await
            .context("Failed to get latest block number")?;
        Ok(block_number)
    }

    pub async fn get_code_at_block(&self, address: Address, block_number: u64) -> Result<Bytes> {
        let code = self
            .provider
            .get_code_at(address)
            .block_id(BlockNumberOrTag::Number(block_number).into())
            .await
            .context("Failed to get code at block")?;
        Ok(code)
    }

    pub async fn get_logs(
        &self,
        from_block: u64,
        to_block: u64,
        contract_address: Address,
        topic0: B256,
    ) -> Result<Vec<Log>> {
        let filter = Filter::new()
            .address(contract_address)
            .event_signature(topic0)
            .from_block(from_block)
            .to_block(to_block);

        let logs = self
            .provider
            .get_logs(&filter)
            .await?;

        Ok(logs)
    }
}
