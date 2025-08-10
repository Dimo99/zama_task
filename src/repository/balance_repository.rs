use alloy_primitives::{Address, U256};
use anyhow::Result;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::str::FromStr;
use tracing::info;

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
