use crate::config::Config;
use crate::deployment::{fetch_token_metadata, find_deployment_block};
use crate::events::{Transfer as EventTransfer, decode_transfer_event};
use crate::insertion_worker::{TransferBatch, run_insertion_worker};
use crate::repository::{Database, Token, TokenRepository, Transfer};
use crate::rpc::RpcClient;
use alloy::sol_types::SolEvent;
use alloy_primitives::{Address, B256};
use anyhow::Result;
use futures::stream::{FuturesOrdered, StreamExt};
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep};
use tracing::{info, warn};

pub struct Scanner {
    client: RpcClient,
    db: Database,
    contract_address: Address,
    transfer_topic: B256,
    batch_size: u64,
    rate_limit_delay_ms: u64,
    max_pending_requests: usize,
}

impl Scanner {
    pub fn new(client: RpcClient, db: Database, config: &Config) -> Result<Self> {
        let transfer_topic = EventTransfer::SIGNATURE_HASH;
        Ok(Scanner {
            client,
            db,
            contract_address: config.erc20_contract_address,
            transfer_topic,
            batch_size: config.batch_size,
            rate_limit_delay_ms: config.rate_limit_delay_ms,
            max_pending_requests: config.max_pending_requests,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let deployment_block = self.ensure_deployment_block().await?;

        let token_repo = TokenRepository::new(&self.db.conn);
        let last_processed_block = token_repo
            .get_last_processed_block(&self.contract_address)?
            .unwrap_or(deployment_block);

        info!("Starting scan from block {}", last_processed_block);

        // Create channel for sending batches to insertion worker
        let (tx, rx) = mpsc::channel::<TransferBatch>(10);

        // Spawn insertion worker
        let db_clone = self.db.clone();
        let contract_address = self.contract_address;
        let insertion_handle =
            tokio::spawn(async move { run_insertion_worker(db_clone, contract_address, rx).await });

        // Set up interval for rate limiting
        let mut rate_limit_interval = interval(Duration::from_millis(self.rate_limit_delay_ms));
        rate_limit_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // Track block ranges to fetch
        let mut next_block_to_fetch = last_processed_block + 1;
        let mut next_block_to_process = last_processed_block + 1;

        // FuturesOrdered to maintain order of results
        let mut pending_fetches = FuturesOrdered::<_>::new();

        loop {
            let latest_block = self.client.get_latest_block().await?;

            // Check if we're caught up
            if next_block_to_fetch > latest_block && pending_fetches.is_empty() {
                info!(
                    "Caught up to latest block {}. Entering polling mode...",
                    latest_block
                );
                sleep(Duration::from_secs(12)).await;
                next_block_to_fetch = next_block_to_process;
                continue;
            }

            tokio::select! {
                // Fire new requests at the rate limit interval
                _ = rate_limit_interval.tick() => {
                    if pending_fetches.len() < self.max_pending_requests && next_block_to_fetch <= latest_block {
                        let from = next_block_to_fetch;
                        let to = (from + self.batch_size - 1).min(latest_block);

                        info!("Firing request for blocks {} to {}", from, to);

                        // Clone what we need for the async task
                        let client = self.client.clone();
                        let contract_address = self.contract_address;
                        let transfer_topic = self.transfer_topic;

                        // Rotate to next RPC for load distribution
                        client.rotate_provider();

                        // Create the future and push it to FuturesOrdered
                        let fetch_future = async move {
                            let rpc_url = client.get_current_url().to_string();
                            let start = Instant::now();
                            let logs = client
                                .get_logs(from, to, contract_address, transfer_topic)
                                .await?;
                            let elapsed = start.elapsed();
                            Ok::<_, anyhow::Error>((from, to, logs, elapsed, rpc_url))
                        };

                        pending_fetches.push_back(fetch_future);
                        next_block_to_fetch = to + 1;
                    }
                }

                // Process results as they come in, in order
                Some(result) = pending_fetches.next() => {
                    let (from, to, logs, elapsed, rpc_url) = result?;

                    info!("Processing {} logs for blocks {} to {} (took {:?} from {})",
                          logs.len(), from, to, elapsed.as_secs_f64(), rpc_url);

                    let mut transfers = Vec::new();

                    for log in &logs {
                        match decode_transfer_event(log) {
                            Ok(event) => {
                                transfers.push(Transfer {
                                    transaction_hash: log.transaction_hash.unwrap(),
                                    log_index: log.log_index.unwrap(),
                                    token_address: self.contract_address,
                                    from_address: event.from,
                                    to_address: event.to,
                                    value: event.value,
                                    block_number: log.block_number.unwrap(),
                                });
                            }
                            Err(e) => {
                                warn!("Failed to decode transfer event: {}", e);
                            }
                        }
                    }

                    // Send batch to insertion worker
                    if !transfers.is_empty() || next_block_to_process <= to {
                        let batch = TransferBatch {
                            transfers,
                            end_block: to,
                        };

                        if tx.send(batch).await.is_err() {
                            warn!("Insertion worker has stopped, exiting...");
                            break;
                        }
                    }

                    next_block_to_process = to + 1;
                }
            }
        }

        // Close channel and wait for insertion worker to finish
        drop(tx);
        insertion_handle.await??;

        Ok(())
    }

    async fn ensure_deployment_block(&self) -> Result<u64> {
        let token_repo = TokenRepository::new(&self.db.conn);
        if let Some(block) = token_repo.get_deployment_block(&self.contract_address)? {
            info!("Using cached deployment block: {}", block);
            return Ok(block);
        }

        info!(
            "Finding deployment block for contract {:?}",
            self.contract_address
        );
        let latest_block = self.client.get_latest_block().await?;
        let deployment_block =
            find_deployment_block(&self.client, self.contract_address, latest_block).await?;

        // Fetch token metadata
        let metadata = fetch_token_metadata(&self.client, self.contract_address).await?;

        let token = Token {
            address: self.contract_address,
            deployment_block,
            last_processed_block: Some(deployment_block),
            name: metadata.name,
            symbol: metadata.symbol,
            decimals: metadata.decimals,
        };

        let token_repo = TokenRepository::new(&self.db.conn);
        token_repo.insert(&token)?;
        Ok(deployment_block)
    }
}
