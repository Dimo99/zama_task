use crate::repository::{Database, TokenRepository, Transfer, TransferRepository};
use alloy_primitives::Address;
use anyhow::Result;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::info;

pub struct TransferBatch {
    pub transfers: Vec<Transfer>,
    pub end_block: u64,
}

pub async fn run_insertion_worker(
    db: Database,
    contract_address: Address,
    mut rx: mpsc::Receiver<TransferBatch>,
) -> Result<()> {
    while let Some(batch) = rx.recv().await {
        let db_clone = db.clone();

        // Use spawn_blocking since database operations are blocking
        tokio::task::spawn_blocking(move || process_batch(db_clone, contract_address, batch))
            .await??;
    }
    Ok(())
}

fn process_batch(db: Database, contract_address: Address, batch: TransferBatch) -> Result<()> {
    let start = Instant::now();

    if !batch.transfers.is_empty() {
        let transfer_repo = TransferRepository::new(&db.conn);
        let inserted = transfer_repo.insert_batch(&batch.transfers)?;
        info!("Inserted {} transfers in {:?}", inserted, start.elapsed());
    }

    // Update last processed block after successful insertion
    let token_repo = TokenRepository::new(&db.conn);
    token_repo.update_last_processed_block(&contract_address, batch.end_block)?;
    info!("Updated last processed block to {}", batch.end_block);

    Ok(())
}
