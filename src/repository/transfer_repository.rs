use super::models::Transfer;
use alloy_primitives::{Address, B256, U256};
use anyhow::Result;
use rusqlite::{Row, ToSql, params, params_from_iter};
use std::str::FromStr;

pub struct TransferRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> TransferRepository<'a> {
    // SQL queries as constants
    const INSERT_TRANSFER: &'static str = "INSERT OR IGNORE INTO transfers (
            transaction_hash, log_index, token_address, 
            from_address, to_address, value, block_number, block_hash, is_finalized
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

    const SELECT_INCOMING_VALUES: &'static str =
        "SELECT value FROM transfers WHERE to_address = ?1";
    const SELECT_OUTGOING_VALUES: &'static str =
        "SELECT value FROM transfers WHERE from_address = ?1";
    const SELECT_UNIQUE_ADDRESSES: &'static str = "SELECT DISTINCT address FROM (
        SELECT to_address as address FROM transfers
        UNION
        SELECT from_address as address FROM transfers
    )";

    const SELECT_TRANSFER_VIEW: &'static str =
        "SELECT transaction_hash, from_address, to_address, value, block_number FROM transfers";

    const UPDATE_FINALITY_STATUS: &'static str =
        "UPDATE transfers SET is_finalized = ?1 WHERE block_number >= ?2 AND block_number <= ?3";

    const DELETE_TRANSFERS_FOR_BLOCK: &'static str =
        "DELETE FROM transfers WHERE block_number = ?1";

    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    fn transfer_params(transfer: &Transfer) -> Vec<Box<dyn ToSql>> {
        vec![
            Box::new(format!("{:?}", transfer.transaction_hash)),
            Box::new(transfer.log_index),
            Box::new(format!("{:?}", transfer.token_address)),
            Box::new(format!("{:?}", transfer.from_address)),
            Box::new(format!("{:?}", transfer.to_address)),
            Box::new(transfer.value.to_string()),
            Box::new(transfer.block_number),
            Box::new(format!("{:?}", transfer.block_hash)),
            Box::new(transfer.is_finalized),
        ]
    }

    pub fn insert(&self, transfer: &Transfer) -> Result<()> {
        let params = Self::transfer_params(transfer);
        self.conn
            .execute(Self::INSERT_TRANSFER, params_from_iter(params))?;
        Ok(())
    }

    pub fn insert_batch(&self, transfers: &[Transfer]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut count = 0;

        {
            let mut stmt = tx.prepare(Self::INSERT_TRANSFER)?;

            for transfer in transfers {
                let params = Self::transfer_params(transfer);
                let result = stmt.execute(params_from_iter(params))?;
                count += result;
            }
        }

        tx.commit()?;
        Ok(count)
    }

    pub fn query_transfers(
        &self,
        from_address: Option<&Address>,
        to_address: Option<&Address>,
        block_range: Option<(u64, u64)>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TransferView>> {
        let mut conditions = Vec::new();
        let mut params: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(from) = from_address {
            conditions.push("from_address = ?");
            params.push(Box::new(format!("{from:?}")));
        }

        if let Some(to) = to_address {
            conditions.push("to_address = ?");
            params.push(Box::new(format!("{to:?}")));
        }

        if let Some((start, end)) = block_range {
            conditions.push("block_number >= ?");
            params.push(Box::new(start));
            conditions.push("block_number <= ?");
            params.push(Box::new(end));
        }

        self.execute_paginated_query(conditions, params, limit, offset, None)
    }

    pub fn get_address_history(
        &self,
        address: &Address,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TransferView>> {
        let address_str = format!("{address:?}");
        let conditions = vec!["(from_address = ? OR to_address = ?)"];
        let params: Vec<Box<dyn ToSql>> =
            vec![Box::new(address_str.clone()), Box::new(address_str)];

        self.execute_paginated_query(conditions, params, limit, offset, None)
    }

    // TODO: Also needs denormalization to perform normally on USDC
    pub fn get_statistics(&self) -> Result<TransferStats> {
        let total_transfers: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM transfers", [], |row| row.get(0))?;

        let unique_addresses: usize = self.conn.query_row(
            "SELECT COUNT(DISTINCT address) FROM (
                SELECT from_address as address FROM transfers
                UNION
                SELECT to_address as address FROM transfers
            )",
            [],
            |row| row.get(0),
        )?;

        let (earliest_block, latest_block): (Option<u64>, Option<u64>) = self.conn.query_row(
            "SELECT MIN(block_number), MAX(block_number) FROM transfers",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        Ok(TransferStats {
            total_transfers,
            unique_addresses,
            earliest_block,
            latest_block,
        })
    }

    // TODO: Could benefit from denormalization
    pub fn get_balance(&self, address: &Address) -> Result<BalanceInfo> {
        let (balance, total_incoming, total_outgoing) = self.calculate_balance(address)?;
        Ok(BalanceInfo {
            balance,
            total_incoming,
            total_outgoing,
        })
    }

    // TODO: detonormalize the database so this works on large tokens as USDC
    pub fn get_top_holders(&self, limit: usize) -> Result<Vec<TokenHolder>> {
        let mut stmt = self.conn.prepare(Self::SELECT_UNIQUE_ADDRESSES)?;
        let addresses: Vec<Address> = stmt
            .query_map([], |row| {
                Address::from_str(&row.get::<_, String>(0)?).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut holders: Vec<TokenHolder> = Vec::new();

        for address in addresses {
            let (balance, _, _) = self.calculate_balance(&address)?;

            if balance > U256::ZERO {
                holders.push(TokenHolder { address, balance });
            }
        }

        holders.sort_by(|a, b| b.balance.cmp(&a.balance));

        holders.truncate(limit);

        Ok(holders)
    }

    fn execute_paginated_query(
        &self,
        conditions: Vec<&str>,
        params: Vec<Box<dyn ToSql>>,
        limit: usize,
        offset: usize,
        order_by: Option<&str>,
    ) -> Result<Vec<TransferView>> {
        let mut query = Self::SELECT_TRANSFER_VIEW.to_string();

        if !conditions.is_empty() {
            query.push_str(" WHERE ");
            query.push_str(&conditions.join(" AND "));
        }

        if let Some(order) = order_by {
            query.push_str(order);
        }

        query.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));

        let mut stmt = self.conn.prepare(&query)?;
        let transfers = stmt
            .query_map(params_from_iter(params), Self::row_to_transfer_view)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transfers)
    }

    fn row_to_transfer_view(row: &Row) -> rusqlite::Result<TransferView> {
        let transaction_hash = row.get::<_, String>(0)?.parse::<B256>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let from_address = Address::from_str(&row.get::<_, String>(1)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let to_address = Address::from_str(&row.get::<_, String>(2)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let value = U256::from_str(&row.get::<_, String>(3)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(TransferView {
            transaction_hash,
            from_address,
            to_address,
            value,
            block_number: row.get(4)?,
        })
    }

    fn calculate_balance(&self, address: &Address) -> Result<(U256, U256, U256)> {
        let address_str = format!("{address:?}");
        let mut stmt = self.conn.prepare(Self::SELECT_INCOMING_VALUES)?;
        let incoming_values = stmt
            .query_map(params![address_str], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let total_incoming = Self::sum_values(incoming_values)?;

        let mut stmt = self.conn.prepare(Self::SELECT_OUTGOING_VALUES)?;
        let outgoing_values = stmt
            .query_map(params![address_str], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let total_outgoing = Self::sum_values(outgoing_values)?;

        let balance = total_incoming.saturating_sub(total_outgoing);

        Ok((balance, total_incoming, total_outgoing))
    }

    fn sum_values(values: Vec<String>) -> Result<U256> {
        let mut total = U256::ZERO;
        for value_str in values {
            let value = U256::from_str(&value_str)
                .map_err(|_| anyhow::anyhow!("Invalid value format in database: {}", value_str))?;
            total = total
                .checked_add(value)
                .ok_or_else(|| anyhow::anyhow!("Overflow in sum calculation"))?;
        }
        Ok(total)
    }

    pub fn get_block_hashes_in_range(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<std::collections::HashMap<u64, B256>> {
        let query = "SELECT DISTINCT block_number, block_hash 
                     FROM transfers 
                     WHERE block_number >= ? AND block_number <= ?";

        let mut stmt = self.conn.prepare(query)?;
        let mut block_hashes = std::collections::HashMap::new();

        let rows = stmt.query_map(params![from_block, to_block], |row| {
            let block_num: u64 = row.get(0)?;
            let block_hash = B256::from_str(&row.get::<_, String>(1)?).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok((block_num, block_hash))
        })?;

        for row in rows {
            let (block_num, block_hash) = row?;
            if let Some(existing_hash) = block_hashes.get(&block_num) {
                if existing_hash != &block_hash {
                    anyhow::bail!(
                        "Block {} has multiple distinct block hashes in DB ({:?} and {:?}), this should be impossible!",
                        block_num,
                        existing_hash,
                        block_hash
                    );
                }
            }
            block_hashes.insert(block_num, block_hash);
        }

        Ok(block_hashes)
    }

    pub fn process_finality_batch(
        &self,
        blocks_to_delete: &[u64],
        transfers_to_insert: &[Transfer],
        mark_finalized_from: u64,
        mark_finalized_to: u64,
    ) -> Result<(usize, usize, usize)> {
        let tx = self.conn.unchecked_transaction()?;

        let mut deleted_count = 0;
        let mut inserted_count = 0;

        for block_num in blocks_to_delete {
            deleted_count += tx.execute(Self::DELETE_TRANSFERS_FOR_BLOCK, params![block_num])?;
        }

        if !transfers_to_insert.is_empty() {
            let mut stmt = tx.prepare(Self::INSERT_TRANSFER)?;
            for transfer in transfers_to_insert {
                let params = Self::transfer_params(transfer);
                let result = stmt.execute(params_from_iter(params))?;
                inserted_count += result;
            }
        }

        // Mark transfers as finalized
        let finalized_count = tx.execute(
            Self::UPDATE_FINALITY_STATUS,
            params![true, mark_finalized_from, mark_finalized_to],
        )?;

        tx.commit()?;

        Ok((deleted_count, inserted_count, finalized_count))
    }
}

#[derive(Debug)]
pub struct TransferView {
    pub transaction_hash: B256,
    pub from_address: Address,
    pub to_address: Address,
    pub value: U256,
    pub block_number: u64,
}

#[derive(Debug)]
pub struct TransferStats {
    pub total_transfers: usize,
    pub unique_addresses: usize,
    pub earliest_block: Option<u64>,
    pub latest_block: Option<u64>,
}

#[derive(Debug)]
pub struct BalanceInfo {
    pub balance: U256,
    pub total_incoming: U256,
    pub total_outgoing: U256,
}

#[derive(Debug)]
pub struct TokenHolder {
    pub address: Address,
    pub balance: U256,
}
