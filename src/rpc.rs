use alloy::providers::fillers::FillProvider;
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::{BlockNumberOrTag, Filter, Log};
use alloy_primitives::{Address, B256, Bytes};
use anyhow::Result;
use regex::Regex;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::timeout;
use tokio_retry::Retry;
use tokio_retry::strategy::{ExponentialBackoff, jitter};
use tracing::{debug, info, warn};

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

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120); // 2 minutes timeout per request

#[derive(Clone)]
pub struct RpcClient {
    providers: Vec<AlloyFullProvider>,
    urls: Vec<String>,
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
            let parsed_url = url
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid RPC URL: {}", url))?;
            let provider: AlloyFullProvider = ProviderBuilder::new().connect_http(parsed_url);
            providers.push(provider);
        }

        Ok(RpcClient {
            providers,
            urls: rpc_urls.to_vec(),
            current_provider: Arc::new(AtomicUsize::new(0)),
            max_retries: 5,
        })
    }

    fn get_provider(&self) -> &AlloyFullProvider {
        let index = self.current_provider.load(Ordering::Relaxed) % self.providers.len();
        &self.providers[index]
    }

    pub fn get_current_url(&self) -> &str {
        let index = self.current_provider.load(Ordering::Relaxed) % self.urls.len();
        &self.urls[index]
    }

    pub fn rotate_provider(&self) {
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
        let current_url = self.get_current_url();
        warn!(
            "RPC error on {}: {}, rotating provider",
            current_url, error_str
        );
        self.rotate_provider();
    }

    fn handle_timeout(&self) -> anyhow::Error {
        let current_url = self.get_current_url();
        warn!(
            "Request timeout after {} seconds on {}, rotating provider",
            REQUEST_TIMEOUT.as_secs(),
            current_url
        );
        self.rotate_provider();
        anyhow::anyhow!(
            "Request timeout after {} seconds",
            REQUEST_TIMEOUT.as_secs()
        )
    }

    pub async fn get_latest_block(&self) -> Result<u64> {
        let client = self.clone();
        Retry::spawn(self.get_retry_strategy(), move || {
            let client = client.clone();
            async move {
                let provider = client.get_provider();
                match timeout(REQUEST_TIMEOUT, provider.get_block_number()).await {
                    Ok(Ok(block_number)) => Ok(block_number),
                    Ok(Err(e)) => {
                        let error_str = e.to_string();
                        client.handle_error(&error_str);
                        Err(anyhow::anyhow!("{}", e))
                    }
                    Err(_) => Err(client.handle_timeout()),
                }
            }
        })
        .await
    }

    pub async fn get_code_at_block(&self, address: Address, block_number: u64) -> Result<Bytes> {
        let client = self.clone();
        Retry::spawn(self.get_retry_strategy(), move || {
            let client = client.clone();
            async move {
                let provider = client.get_provider();
                let future = provider
                    .get_code_at(address)
                    .block_id(BlockNumberOrTag::Number(block_number).into());

                match timeout(REQUEST_TIMEOUT, future).await {
                    Ok(Ok(result)) => Ok(result),
                    Ok(Err(e)) => {
                        let error_str = e.to_string();
                        client.handle_error(&error_str);
                        Err(anyhow::anyhow!("{}", e))
                    }
                    Err(_) => Err(client.handle_timeout()),
                }
            }
        })
        .await
    }

    async fn get_logs_internal(
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
                let provider = client.get_provider();
                let filter = Filter::new()
                    .address(contract_address)
                    .event_signature(topic0)
                    .from_block(from_block)
                    .to_block(to_block);

                match timeout(REQUEST_TIMEOUT, provider.get_logs(&filter)).await {
                    Ok(Ok(logs)) => Ok(Ok(logs)),
                    Ok(Err(e)) => {
                        let error_str = e.to_string();

                        if error_str.contains("exceeds max results") {
                            debug!(
                                "Max results exceeded for blocks {}-{}, will split range",
                                from_block, to_block
                            );
                            // hack since we don't want to retry on this specific error
                            Ok(Err(anyhow::anyhow!("{}", e)))
                        } else {
                            client.handle_error(&error_str);
                            Err(anyhow::anyhow!("{}", e))
                        }
                    }
                    Err(_) => Err(client.handle_timeout()),
                }
            }
        })
        .await
        .and_then(|r| r)
    }

    fn parse_max_results_error(error_str: &str) -> Option<(u64, u64)> {
        let re = Regex::new(r"retry with the range (\d+)-(\d+)").ok()?;
        let captures = re.captures(error_str)?;

        let from = captures.get(1)?.as_str().parse().ok()?;
        let to = captures.get(2)?.as_str().parse().ok()?;

        Some((from, to))
    }

    pub async fn get_logs(
        &self,
        from_block: u64,
        to_block: u64,
        contract_address: Address,
        topic0: B256,
    ) -> Result<Vec<Log>> {
        let mut all_logs = Vec::new();
        let mut current_from = from_block;

        while current_from <= to_block {
            let current_to = to_block;

            match self
                .get_logs_internal(current_from, current_to, contract_address, topic0)
                .await
            {
                Ok(logs) => {
                    all_logs.extend(logs);
                    break;
                }
                Err(e) => {
                    let error_str = e.to_string();

                    if error_str.contains("exceeds max results") {
                        if let Some((suggested_from, suggested_to)) =
                            Self::parse_max_results_error(&error_str)
                        {
                            info!(
                                "Hit max results limit for blocks {}-{}, splitting at block {}",
                                current_from, current_to, suggested_to
                            );

                            let logs = self
                                .get_logs_internal(
                                    suggested_from,
                                    suggested_to,
                                    contract_address,
                                    topic0,
                                )
                                .await?;

                            all_logs.extend(logs);
                            current_from = suggested_to + 1;
                        } else {
                            return Err(e);
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        Ok(all_logs)
    }
}
