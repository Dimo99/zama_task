use super::models::Transfer;
use anyhow::Result;
use rusqlite::params;

pub struct TransferRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> TransferRepository<'a> {
    // SQL queries as constants
    const INSERT_TRANSFER: &'static str = "INSERT OR IGNORE INTO transfers (
            transaction_hash, log_index, token_address, 
            from_address, to_address, value, block_number
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    #[allow(dead_code)]
    pub fn insert(&self, transfer: &Transfer) -> Result<()> {
        self.conn.execute(
            Self::INSERT_TRANSFER,
            params![
                transfer.transaction_hash,
                transfer.log_index,
                format!("{:?}", transfer.token_address),
                format!("{:?}", transfer.from_address),
                format!("{:?}", transfer.to_address),
                transfer.value,
                transfer.block_number,
            ],
        )?;
        Ok(())
    }

    pub fn insert_batch(&self, transfers: &[Transfer]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut count = 0;

        {
            let mut stmt = tx.prepare(Self::INSERT_TRANSFER)?;

            for transfer in transfers {
                let result = stmt.execute(params![
                    transfer.transaction_hash,
                    transfer.log_index,
                    format!("{:?}", transfer.token_address),
                    format!("{:?}", transfer.from_address),
                    format!("{:?}", transfer.to_address),
                    transfer.value,
                    transfer.block_number,
                ])?;
                count += result;
            }
        }

        tx.commit()?;
        Ok(count)
    }
}
