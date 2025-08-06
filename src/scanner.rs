use crate::deployment::find_deployment_block;
use crate::events::{Transfer as EventTransfer, decode_transfer_event};
use crate::insertion_worker::{run_insertion_worker, TransferBatch};
use crate::repository::{Database, Token, Transfer, TokenRepository};
use crate::rpc::RpcClient;
use alloy::sol_types::SolEvent;
use alloy_primitives::{Address, B256};
use anyhow::Result;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{info, warn};

const BATCH_SIZE: u64 = 1000; // Most public RPCs allow up to 1k logs per request, empirically proven
const RATE_LIMIT_DELAY_MS: u64 = 200; // 200ms between requests = 5 requests per second

pub struct Scanner {
    client: RpcClient,
    db: Database,
    contract_address: Address,
    transfer_topic: B256,
}

impl Scanner {
    pub fn new(client: RpcClient, db: Database, contract_address: Address) -> Result<Self> {
        let transfer_topic = EventTransfer::SIGNATURE_HASH;
        Ok(Scanner {
            client,
            db,
            contract_address,
            transfer_topic,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let deployment_block = self.ensure_deployment_block().await?;

        let token_repo = TokenRepository::new(&self.db.conn);
        let mut last_processed_block = token_repo
            .get_last_processed_block(&self.contract_address)?
            .unwrap_or(deployment_block);

        info!("Starting scan from block {}", last_processed_block);
        
        // Create channel for sending batches to insertion worker
        let (tx, rx) = mpsc::channel::<TransferBatch>(10);
        
        // Spawn insertion worker
        let db_clone = self.db.clone();
        let contract_address = self.contract_address;
        let insertion_handle = tokio::spawn(async move {
            run_insertion_worker(db_clone, contract_address, rx).await
        });

        loop {
            let loop_start = Instant::now();
            
            let latest_block = self.client.get_latest_block().await?;

            if last_processed_block >= latest_block {
                info!(
                    "Caught up to latest block {}. Entering polling mode...",
                    latest_block
                );
                sleep(Duration::from_secs(12)).await;
                continue;
            }

            let to_block = (last_processed_block + BATCH_SIZE).min(latest_block);

            let from = last_processed_block + 1;
            info!("Fetching logs for blocks {} to {}", from, to_block);

            let logs = match self
                .client
                .get_logs(from, to_block, self.contract_address, self.transfer_topic)
                .await
            {
                Ok(logs) => logs,
                Err(e) if e.to_string().contains("429") => {
                    warn!("Rate limited, waiting 1 second before retry...");
                    sleep(Duration::from_secs(1)).await;
                    self.client
                        .get_logs(from, to_block, self.contract_address, self.transfer_topic)
                        .await?
                }
                Err(e) => return Err(e),
            };
            
            info!("Received {} logs for blocks {} to {}", logs.len(), from, to_block);

            let mut transfers = Vec::new();
            
            for log in &logs {
                match decode_transfer_event(log) {
                    Ok(event) => {
                        transfers.push(Transfer {
                            transaction_hash: format!("{:?}", log.transaction_hash.unwrap()),
                            log_index: log.log_index.unwrap(),
                            token_address: self.contract_address,
                            from_address: event.from,
                            to_address: event.to,
                            value: event.value.to_string(),
                            block_number: log.block_number.unwrap(),
                        });
                    }
                    Err(e) => {
                        warn!("Failed to decode transfer event: {}", e);
                    }
                }
            }

            // Send batch to insertion worker
            if !transfers.is_empty() || last_processed_block < to_block {
                let batch = TransferBatch {
                    transfers,
                    end_block: to_block,
                };
                
                if tx.send(batch).await.is_err() {
                    warn!("Insertion worker has stopped, exiting...");
                    break;
                }
            }

            last_processed_block = to_block;
            
            // Smart rate limiting: ensure minimum time between loop iterations
            let loop_duration = loop_start.elapsed();
            let target_duration = Duration::from_millis(RATE_LIMIT_DELAY_MS);
            if loop_duration < target_duration {
                sleep(target_duration - loop_duration).await;
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

        let token = Token {
            address: self.contract_address,
            deployment_block,
            last_processed_block: Some(deployment_block),
        };
        
        let token_repo = TokenRepository::new(&self.db.conn);
        token_repo.insert(&token)?;
        Ok(deployment_block)
    }
}
