use crate::config::Config;
use crate::deployment::{fetch_token_metadata, find_deployment_block};
use crate::events::{Transfer as EventTransfer, decode_transfer_event};
use crate::insertion_worker::{TransferBatch, run_insertion_worker};
use crate::repository::{
    BalanceRepository, Database, Token, TokenRepository, Transfer, TransferRepository,
};
use crate::rpc::RpcClient;
use alloy::sol_types::SolEvent;
use alloy_primitives::{Address, B256};
use anyhow::Result;
use futures::stream::{FuturesOrdered, StreamExt};
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, info, warn};

pub struct Scanner {
    client: RpcClient,
    db: Database,
    contract_address: Address,
    transfer_topic: B256,
    batch_size: u64,
    rate_limit_delay_ms: u64,
    max_pending_requests: usize,
    finality_update_interval_secs: u64,
    block_time_secs: u64,
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
            finality_update_interval_secs: config.finality_update_interval_secs,
            block_time_secs: config.block_time_secs,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let deployment_block = self.ensure_deployment_block().await?;

        let token_repo = TokenRepository::new(&self.db.conn);
        let last_processed_block = token_repo
            .get_last_processed_block(&self.contract_address)?
            .unwrap_or(deployment_block);

        info!("Starting scan from block {}", last_processed_block);

        // Do initial finality update before starting main loop
        info!("Performing initial finality update...");
        if let Err(e) = self.update_finality(true).await {
            error!("Initial finality update failed: {}", e);
        }

        // Create channel for sending batches to insertion worker
        let (tx, rx) = mpsc::channel::<TransferBatch>(10);

        // Spawn insertion worker
        let db_clone = self.db.clone();
        let contract_address = self.contract_address;
        let insertion_handle =
            tokio::spawn(async move { run_insertion_worker(db_clone, contract_address, rx).await });

        let mut rate_limit_interval = interval(Duration::from_millis(self.rate_limit_delay_ms));
        rate_limit_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let mut finality_interval =
            interval(Duration::from_secs(self.finality_update_interval_secs));
        finality_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut block_poll_interval = interval(Duration::from_secs(self.block_time_secs));
        block_poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut next_block_to_fetch = last_processed_block + 1;
        let mut next_block_to_process = last_processed_block + 1;

        let mut pending_fetches = FuturesOrdered::<_>::new();

        loop {
            let latest_block = self.client.get_latest_block().await?;

            if next_block_to_fetch > latest_block && pending_fetches.is_empty() {
                info!(
                    "Caught up to latest block {}. Waiting for new blocks...",
                    latest_block
                );
                block_poll_interval.tick().await;
                next_block_to_fetch = next_block_to_process;
                continue;
            }

            tokio::select! {
                // Periodically update finality
                _ = finality_interval.tick() => {
                    if let Err(e) = self.update_finality(false).await {
                        error!("Failed to update finality: {}", e);
                    }
                }

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
                                let block_num = log.block_number.unwrap();
                                transfers.push(Transfer {
                                    transaction_hash: log.transaction_hash.unwrap(),
                                    log_index: log.log_index.unwrap(),
                                    token_address: self.contract_address,
                                    from_address: event.from,
                                    to_address: event.to,
                                    value: event.value,
                                    block_number: block_num,
                                    block_hash: log.block_hash.unwrap(),
                                    is_finalized: self.should_mark_as_finalized(block_num),
                                });
                            }
                            Err(e) => {
                                anyhow::bail!("Failed to decode transfer event: {}", e);
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
            last_processed_finalized_block: Some(deployment_block),
            name: metadata.name,
            symbol: metadata.symbol,
            decimals: metadata.decimals,
        };

        let token_repo = TokenRepository::new(&self.db.conn);
        token_repo.insert(&token)?;
        Ok(deployment_block)
    }

    async fn update_finality(&self, is_initial: bool) -> Result<()> {
        let token_repo = TokenRepository::new(&self.db.conn);
        let transfer_repo = TransferRepository::new(&self.db.conn);

        let last_finalized = token_repo
            .get_last_processed_finalized_block(&self.contract_address)?
            .unwrap_or(0);

        let last_processed = token_repo
            .get_last_processed_block(&self.contract_address)?
            .unwrap_or(0);

        let current_finalized = self.client.get_finalized_block().await?;

        // Always process blocks only up to min(last_processed, current_finalized)
        let target_finalized = current_finalized.min(last_processed);

        if target_finalized <= last_finalized {
            // No new blocks to finalize
            return Ok(());
        }

        info!(
            "Updating finality from block {} to {} (chain finalized: {}, last processed: {})",
            last_finalized + 1,
            target_finalized,
            current_finalized,
            last_processed
        );

        let mut current_from = last_finalized + 1;

        while current_from <= target_finalized {
            let current_to = (current_from + self.batch_size - 1).min(target_finalized);

            let chain_logs = self
                .client
                .get_logs(
                    current_from,
                    current_to,
                    self.contract_address,
                    self.transfer_topic,
                )
                .await?;

            let stored_block_hashes =
                transfer_repo.get_block_hashes_in_range(current_from, current_to)?;

            let mut chain_block_hashes: std::collections::HashMap<u64, B256> =
                std::collections::HashMap::new();
            let mut chain_transfers: Vec<Transfer> = Vec::new();

            for log in &chain_logs {
                match decode_transfer_event(log) {
                    Ok(event) => {
                        let block_num = log.block_number.unwrap();
                        let block_hash = log.block_hash.unwrap();

                        chain_block_hashes.insert(block_num, block_hash);

                        chain_transfers.push(Transfer {
                            transaction_hash: log.transaction_hash.unwrap(),
                            log_index: log.log_index.unwrap(),
                            token_address: self.contract_address,
                            from_address: event.from,
                            to_address: event.to,
                            value: event.value,
                            block_number: block_num,
                            block_hash,
                            is_finalized: true,
                        });
                    }
                    Err(e) => {
                        anyhow::bail!("Failed to decode transfer event: {}", e);
                    }
                }
            }

            // Find blocks that need reprocessing
            let mut blocks_to_reprocess = std::collections::HashSet::new();

            // Check each block that has transfers on chain
            for (block_num, chain_hash) in &chain_block_hashes {
                match stored_block_hashes.get(block_num) {
                    Some(stored_hash) if stored_hash != chain_hash => {
                        warn!(
                            "Reorg detected at block {}! Hash mismatch: chain {:?} vs stored {:?}",
                            block_num, chain_hash, stored_hash
                        );
                        blocks_to_reprocess.insert(*block_num);
                    }
                    None => {
                        warn!("Block {} has transfers on chain but not in DB", block_num);
                        blocks_to_reprocess.insert(*block_num);
                    }
                    _ => {} // Hashes match, all good
                }
            }

            // Check for blocks that exist in DB but not on chain
            for block_num in stored_block_hashes.keys() {
                if !chain_block_hashes.contains_key(block_num) {
                    warn!("Block {} has transfers in DB but not on chain", block_num);
                    blocks_to_reprocess.insert(*block_num);
                }
            }

            let mut transfers_to_insert = Vec::new();
            for block_num in &blocks_to_reprocess {
                transfers_to_insert.extend(
                    chain_transfers
                        .iter()
                        .filter(|t| t.block_number == *block_num)
                        .cloned(),
                );
            }

            let blocks_to_delete: Vec<u64> = blocks_to_reprocess.into_iter().collect();
            let (deleted, inserted, finalized) = transfer_repo.process_finality_batch(
                &blocks_to_delete,
                &transfers_to_insert,
                current_from,
                current_to,
            )?;

            if deleted > 0 {
                info!(
                    "Deleted {} transfers from {} reorged blocks",
                    deleted,
                    blocks_to_delete.len()
                );
            }
            if inserted > 0 {
                info!("Inserted {} transfers during finality update", inserted);
            }
            if finalized > 0 {
                info!(
                    "Marked {} transfers as finalized in blocks {}-{}",
                    finalized, current_from, current_to
                );
            }

            // Apply balance updates - transfers_to_insert are all finalized
            // and chain_transfers contains all transfers in the range (including those just marked as finalized)
            if !chain_transfers.is_empty() {
                let balance_repo = BalanceRepository::new(&self.db.conn);
                balance_repo.apply_transfers(&chain_transfers)?;
                info!(
                    "Applied balance updates for {} finalized transfers",
                    chain_transfers.len()
                );
            }

            current_from = current_to + 1;
        }

        // Update last_processed_finalized_block at the very end in a separate transaction
        // For initial update, we can set to current_finalized since no concurrent processes
        // For runtime updates, only update to target_finalized to avoid race conditions
        let update_to = if is_initial {
            current_finalized
        } else {
            target_finalized
        };

        token_repo.update_last_processed_finalized_block(&self.contract_address, update_to)?;
        info!("Updated last processed finalized block to {}", update_to);

        Ok(())
    }

    pub fn should_mark_as_finalized(&self, block_number: u64) -> bool {
        let token_repo = TokenRepository::new(&self.db.conn);

        if let Ok(Some(last_finalized)) =
            token_repo.get_last_processed_finalized_block(&self.contract_address)
        {
            block_number <= last_finalized
        } else {
            false
        }
    }
}
