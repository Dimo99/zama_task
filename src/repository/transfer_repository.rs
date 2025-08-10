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
            from_address, to_address, value, block_number
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

    const SELECT_INCOMING_VALUES: &'static str =
        "SELECT value FROM transfers WHERE to_address = ?1";
    const SELECT_OUTGOING_VALUES: &'static str =
        "SELECT value FROM transfers WHERE from_address = ?1";
    const SELECT_UNIQUE_ADDRESSES: &'static str = "SELECT DISTINCT address FROM (
        SELECT to_address as address FROM transfers
        UNION
        SELECT from_address as address FROM transfers
    )";

    const SELECT_TRANSFER: &'static str = "SELECT transaction_hash, log_index, token_address, from_address, to_address, value, block_number FROM transfers";

    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, transfer: &Transfer) -> Result<()> {
        self.conn.execute(
            Self::INSERT_TRANSFER,
            params![
                format!("{:?}", transfer.transaction_hash),
                transfer.log_index,
                format!("{:?}", transfer.token_address),
                format!("{:?}", transfer.from_address),
                format!("{:?}", transfer.to_address),
                transfer.value.to_string(),
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
                    format!("{:?}", transfer.transaction_hash),
                    transfer.log_index,
                    format!("{:?}", transfer.token_address),
                    format!("{:?}", transfer.from_address),
                    format!("{:?}", transfer.to_address),
                    transfer.value.to_string(),
                    transfer.block_number,
                ])?;
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
    ) -> Result<Vec<Transfer>> {
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
    ) -> Result<Vec<Transfer>> {
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
    ) -> Result<Vec<Transfer>> {
        let mut query = Self::SELECT_TRANSFER.to_string();

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
            .query_map(params_from_iter(params), Self::row_to_transfer)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transfers)
    }

    fn row_to_transfer(row: &Row) -> rusqlite::Result<Transfer> {
        let transaction_hash = row.get::<_, String>(0)?.parse::<B256>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let token_address = Address::from_str(&row.get::<_, String>(2)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let from_address = Address::from_str(&row.get::<_, String>(3)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let to_address = Address::from_str(&row.get::<_, String>(4)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let value = U256::from_str(&row.get::<_, String>(5)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(Transfer {
            transaction_hash,
            log_index: row.get(1)?,
            token_address,
            from_address,
            to_address,
            value,
            block_number: row.get(6)?,
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
