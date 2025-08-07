use alloy::providers::fillers::FillProvider;
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::{BlockNumberOrTag, Filter, Log};
use alloy_primitives::{Address, B256, Bytes};
use anyhow::Result;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_retry::strategy::{jitter, ExponentialBackoff};
use tokio_retry::Retry;
use tracing::{debug, warn};

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

#[derive(Clone)]
pub struct RpcClient {
    providers: Vec<AlloyFullProvider>,
    current_provider: Arc<AtomicUsize>,
    max_retries: usize,
}

impl RpcClient {
    pub fn new(rpc_urls: &[String]) -> Result<Self> {
        if rpc_urls.is_empty() {
            return Err(anyhow::anyhow!("At least one RPC URL must be provided"));
        }

        let mut providers = Vec::new();
        for url in rpc_urls {
            let parsed_url = url.parse()
                .map_err(|_| anyhow::anyhow!("Invalid RPC URL: {}", url))?;
            let provider: AlloyFullProvider = ProviderBuilder::new().connect_http(parsed_url);
            providers.push(provider);
        }

        Ok(RpcClient {
            providers,
            current_provider: Arc::new(AtomicUsize::new(0)),
            max_retries: 5,
        })
    }

    fn get_provider(&self) -> &AlloyFullProvider {
        let index = self.current_provider.load(Ordering::Relaxed) % self.providers.len();
        &self.providers[index]
    }

    fn rotate_provider(&self) {
        let current = self.current_provider.load(Ordering::Relaxed);
        let next = (current + 1) % self.providers.len();
        self.current_provider.store(next, Ordering::Relaxed);
        
        if self.providers.len() > 1 {
            debug!("Rotating to RPC provider #{}", next);
        }
    }

    fn get_retry_strategy(&self) -> impl Iterator<Item = Duration> {
        ExponentialBackoff::from_millis(100)
            .factor(2)
            .max_delay(Duration::from_secs(10))
            .map(jitter)
            .take(self.max_retries)
    }

    fn handle_error(&self, error_str: &str) {
        // Check for rate limiting
        if error_str.contains("429") || error_str.contains("rate") {
            warn!("Rate limited on current RPC, rotating provider");
            self.rotate_provider();
        }
        // Check for connection errors
        else if error_str.contains("connection") || 
                error_str.contains("timeout") ||
                error_str.contains("refused") {
            warn!("Connection error on current RPC, rotating provider");
            self.rotate_provider();
        }
    }

    pub async fn get_latest_block(&self) -> Result<u64> {
        let client = self.clone();
        Retry::spawn(self.get_retry_strategy(), move || {
            let client = client.clone();
            async move {
                client.get_provider()
                    .get_block_number()
                    .await
                    .map_err(|e| {
                        let error_str = e.to_string();
                        client.handle_error(&error_str);
                        anyhow::anyhow!("{}", e)
                    })
            }
        })
        .await
    }

    pub async fn get_code_at_block(&self, address: Address, block_number: u64) -> Result<Bytes> {
        let client = self.clone();
        Retry::spawn(self.get_retry_strategy(), move || {
            let client = client.clone();
            async move {
                client.get_provider()
                    .get_code_at(address)
                    .block_id(BlockNumberOrTag::Number(block_number).into())
                    .await
                    .map_err(|e| {
                        let error_str = e.to_string();
                        client.handle_error(&error_str);
                        anyhow::anyhow!("{}", e)
                    })
            }
        })
        .await
    }

    pub async fn get_logs(
        &self,
        from_block: u64,
        to_block: u64,
        contract_address: Address,
        topic0: B256,
    ) -> Result<Vec<Log>> {
        let client = self.clone();
        Retry::spawn(self.get_retry_strategy(), move || {
            let client = client.clone();
            async move {
                let filter = Filter::new()
                    .address(contract_address)
                    .event_signature(topic0)
                    .from_block(from_block)
                    .to_block(to_block);

                client.get_provider()
                    .get_logs(&filter)
                    .await
                    .map_err(|e| {
                        let error_str = e.to_string();
                        client.handle_error(&error_str);
                        anyhow::anyhow!("{}", e)
                    })
            }
        })
        .await
    }
}