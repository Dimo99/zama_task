use alloy_primitives::{Address, U256};
use anyhow::Result;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::str::FromStr;
use tracing::info;

use crate::repository::Transfer;

pub struct BalanceRepository<'a> {
    conn: &'a Connection,
}

impl<'a> BalanceRepository<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Pad a U256 balance to 78 digits for proper sorting
    /// U256 max is approximately 10^77, so 78 digits is sufficient
    pub fn pad_balance(balance: &U256) -> String {
        format!("{balance:0>78}")
    }

    /// Update balance for a single address
    pub fn update_balance(&self, address: &Address, balance: &U256) -> Result<()> {
        let address_str = format!("{address:?}");
        let padded = Self::pad_balance(balance);

        self.conn.execute(
            "INSERT OR REPLACE INTO balances (address, balance_padded) VALUES (?1, ?2)",
            params![address_str, padded],
        )?;

        Ok(())
    }

    /// Apply incremental balance updates from new transfers
    /// Much more efficient than recalculating from scratch
    pub fn apply_transfers(&self, transfers: &[Transfer]) -> Result<()> {
        if transfers.is_empty() {
            return Ok(());
        }

        let mut balance_increases: HashMap<Address, U256> = HashMap::new();
        let mut balance_decreases: HashMap<Address, U256> = HashMap::new();

        for transfer in transfers {
            if !transfer.is_finalized {
                continue;
            }

            *balance_increases
                .entry(transfer.to_address)
                .or_insert(U256::ZERO) += transfer.value;

            *balance_decreases
                .entry(transfer.from_address)
                .or_insert(U256::ZERO) += transfer.value;
        }

        let tx = self.conn.unchecked_transaction()?;

        // TODO: Optimize by batch fetching all current balances in a single query
        // instead of individual queries per address. For batches with many addresses,
        // we could use WHERE address IN (?, ?, ...) with chunking to respect SQL limits.
        // Current approach is fine for typical batches but could be improved for large ones.
        for (address, increase) in &balance_increases {
            let address_str = format!("{address:?}");

            let current: Option<String> = tx
                .query_row(
                    "SELECT balance_padded FROM balances WHERE address = ?1",
                    params![&address_str],
                    |row| row.get(0),
                )
                .ok();

            let mut balance = match current {
                Some(padded) => {
                    let trimmed = padded.trim_start_matches('0');
                    if trimmed.is_empty() {
                        U256::ZERO
                    } else {
                        U256::from_str(trimmed)?
                    }
                }
                None => U256::ZERO,
            };

            balance = balance.wrapping_add(*increase);

            if let Some(decrease) = balance_decreases.get(address) {
                balance = balance.saturating_sub(*decrease);
            }

            if balance > U256::ZERO {
                let padded = Self::pad_balance(&balance);
                tx.execute(
                    "INSERT OR REPLACE INTO balances (address, balance_padded) VALUES (?1, ?2)",
                    params![address_str, padded],
                )?;
            } else {
                // Remove zero balances
                tx.execute(
                    "DELETE FROM balances WHERE address = ?1",
                    params![address_str],
                )?;
            }
        }

        // Handle addresses that only sent (not received)
        for (address, decrease) in balance_decreases {
            if balance_increases.contains_key(&address) {
                continue; // Already handled above
            }

            let address_str = format!("{address:?}");

            // Get current balance
            let current: Option<String> = tx
                .query_row(
                    "SELECT balance_padded FROM balances WHERE address = ?1",
                    params![&address_str],
                    |row| row.get(0),
                )
                .ok();

            match current {
                Some(padded) => {
                    let trimmed = padded.trim_start_matches('0');
                    let balance = if trimmed.is_empty() {
                        U256::ZERO
                    } else {
                        U256::from_str(trimmed)?
                    };

                    let new_balance = balance.wrapping_sub(decrease);

                    if new_balance > U256::ZERO {
                        let padded = Self::pad_balance(&new_balance);
                        tx.execute(
                            "INSERT OR REPLACE INTO balances (address, balance_padded) VALUES (?1, ?2)",
                            params![address_str, padded],
                        )?;
                    } else {
                        tx.execute(
                            "DELETE FROM balances WHERE address = ?1",
                            params![address_str],
                        )?;
                    }
                }
                None => {
                    // Address has no balance but is sending - this shouldn't happen with finalized transfers
                }
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Update balances for addresses affected by new finalized transfers
    /// This recalculates balances from scratch for the given addresses
    pub fn update_balances_for_addresses(
        &self,
        conn: &Connection,
        addresses: &[Address],
    ) -> Result<()> {
        if addresses.is_empty() {
            return Ok(());
        }

        let mut balances = HashMap::new();

        for address in addresses {
            let address_str = format!("{address:?}");

            // Calculate balance from all finalized transfers
            // Get incoming values
            let mut stmt = conn
                .prepare("SELECT value FROM transfers WHERE to_address = ? AND is_finalized = 1")?;
            let incoming_values = stmt
                .query_map(params![address_str], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;

            let mut total_incoming = U256::ZERO;
            for value_str in incoming_values {
                let value = U256::from_str(&value_str)
                    .map_err(|_| anyhow::anyhow!("Invalid value format: {}", value_str))?;
                total_incoming = total_incoming.wrapping_add(value);
            }

            // Get outgoing values
            let mut stmt = conn.prepare(
                "SELECT value FROM transfers WHERE from_address = ? AND is_finalized = 1",
            )?;
            let outgoing_values = stmt
                .query_map(params![address_str], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;

            let mut total_outgoing = U256::ZERO;
            for value_str in outgoing_values {
                let value = U256::from_str(&value_str)
                    .map_err(|_| anyhow::anyhow!("Invalid value format: {}", value_str))?;
                total_outgoing = total_outgoing.wrapping_add(value);
            }

            let balance = total_incoming.saturating_sub(total_outgoing);

            // Only store non-zero balances
            if balance > U256::ZERO {
                balances.insert(*address, balance);
            } else {
                // Delete zero balances
                self.conn.execute(
                    "DELETE FROM balances WHERE address = ?",
                    params![address_str],
                )?;
            }
        }

        // Update all non-zero balances
        if !balances.is_empty() {
            self.update_balances_batch(&balances)?;
        }

        Ok(())
    }

    /// Update multiple balances in a single transaction
    pub fn update_balances_batch(&self, balances: &HashMap<Address, U256>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO balances (address, balance_padded) VALUES (?1, ?2)",
            )?;

            for (address, balance) in balances {
                let address_str = format!("{address:?}");
                let padded = Self::pad_balance(balance);
                stmt.execute(params![address_str, padded])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Get balance for an address (returns U256::ZERO if not found)
    pub fn get_balance(&self, address: &Address) -> Result<U256> {
        let address_str = format!("{address:?}");

        let padded: Option<String> = self
            .conn
            .query_row(
                "SELECT balance_padded FROM balances WHERE address = ?1",
                params![address_str],
                |row| row.get(0),
            )
            .ok();

        match padded {
            Some(p) => {
                // Remove leading zeros and parse
                let trimmed = p.trim_start_matches('0');
                if trimmed.is_empty() {
                    Ok(U256::ZERO)
                } else {
                    U256::from_str(trimmed)
                        .map_err(|_| anyhow::anyhow!("Invalid balance format in database"))
                }
            }
            None => Ok(U256::ZERO),
        }
    }

    /// Get top holders sorted by balance
    pub fn get_top_holders(&self, limit: usize) -> Result<Vec<(Address, U256)>> {
        let mut stmt = self.conn.prepare(
            "SELECT address, balance_padded FROM balances 
             ORDER BY balance_padded DESC 
             LIMIT ?1",
        )?;

        let holders = stmt
            .query_map(params![limit], |row| {
                let address_str: String = row.get(0)?;
                let padded: String = row.get(1)?;

                let address = Address::from_str(&address_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;

                // Remove leading zeros and parse
                let trimmed = padded.trim_start_matches('0');
                let balance = if trimmed.is_empty() {
                    U256::ZERO
                } else {
                    U256::from_str(trimmed).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?
                };

                Ok((address, balance))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(holders)
    }

    /// Populate initial balances from existing transfers
    /// This is used during migration to build the initial balance table
    pub fn populate_from_transfers(&self, conn: &Connection) -> Result<()> {
        info!("Loading all finalized transfers into memory...");

        let mut balances: HashMap<Address, U256> = HashMap::new();

        // Load all transfers in one query and process in memory
        let mut stmt = conn.prepare(
            "SELECT from_address, to_address, value 
             FROM transfers 
             WHERE is_finalized = 1",
        )?;

        let mut count = 0;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, // from_address
                row.get::<_, String>(1)?, // to_address
                row.get::<_, String>(2)?, // value
            ))
        })?;

        for row in rows {
            let (from_str, to_str, value_str) = row?;

            let from_address = Address::from_str(&from_str)?;
            let to_address = Address::from_str(&to_str)?;
            let value = U256::from_str(&value_str)
                .map_err(|_| anyhow::anyhow!("Invalid value format: {}", value_str))?;

            // Subtract from sender
            let from_balance = balances.entry(from_address).or_insert(U256::ZERO);
            *from_balance = from_balance.wrapping_sub(value);

            // Add to receiver
            let to_balance = balances.entry(to_address).or_insert(U256::ZERO);
            *to_balance = to_balance.wrapping_add(value);

            count += 1;
            if count % 100_000 == 0 {
                info!("Processed {} transfers...", count);
            }
        }

        info!("Processed {} total transfers", count);
        info!("Calculated balances for {} addresses", balances.len());

        // Filter out zero balances
        let non_zero_balances: HashMap<Address, U256> = balances
            .into_iter()
            .filter(|(_, balance)| *balance > U256::ZERO)
            .collect();

        info!(
            "{} addresses have non-zero balances",
            non_zero_balances.len()
        );

        // Insert balances in batches
        const BATCH_SIZE: usize = 10_000;
        let total = non_zero_balances.len();
        let all_addresses: Vec<Address> = non_zero_balances.keys().cloned().collect();

        for (batch_idx, chunk) in all_addresses.chunks(BATCH_SIZE).enumerate() {
            let mut batch = HashMap::new();
            for addr in chunk {
                if let Some(balance) = non_zero_balances.get(addr) {
                    batch.insert(*addr, *balance);
                }
            }

            self.update_balances_batch(&batch)?;

            let processed = ((batch_idx + 1) * BATCH_SIZE).min(total);
            info!(
                "Inserted {}/{} balances ({:.1}% complete)",
                processed,
                total,
                (processed as f64 / total as f64) * 100.0
            );
        }

        info!("Balance migration completed successfully");
        Ok(())
    }
}
