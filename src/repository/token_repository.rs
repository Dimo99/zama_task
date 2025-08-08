use super::models::Token;
use alloy_primitives::Address;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};

pub struct TokenRepository<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> TokenRepository<'a> {
    // SQL queries as constants for better maintainability
    const INSERT_TOKEN: &'static str =
        "INSERT OR IGNORE INTO tokens (address, deployment_block, last_processed_block) 
         VALUES (?1, ?2, ?3)";

    const UPDATE_LAST_PROCESSED_BLOCK: &'static str =
        "UPDATE tokens SET last_processed_block = ?1 WHERE address = ?2";

    const GET_DEPLOYMENT_BLOCK: &'static str =
        "SELECT deployment_block FROM tokens WHERE address = ?1";

    const GET_LAST_PROCESSED_BLOCK: &'static str =
        "SELECT last_processed_block FROM tokens WHERE address = ?1";

    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, token: &Token) -> Result<()> {
        self.conn.execute(
            Self::INSERT_TOKEN,
            params![
                format!("{:?}", token.address),
                token.deployment_block,
                token.last_processed_block.unwrap_or(token.deployment_block)
            ],
        )?;
        Ok(())
    }

    pub fn get_deployment_block(&self, address: &Address) -> Result<Option<u64>> {
        let block: Option<u64> = self
            .conn
            .query_row(
                Self::GET_DEPLOYMENT_BLOCK,
                params![format!("{:?}", address)],
                |row| row.get(0),
            )
            .optional()?;
        Ok(block)
    }

    pub fn get_last_processed_block(&self, address: &Address) -> Result<Option<u64>> {
        let block: Option<u64> = self
            .conn
            .query_row(
                Self::GET_LAST_PROCESSED_BLOCK,
                params![format!("{:?}", address)],
                |row| row.get(0),
            )
            .optional()?;
        Ok(block)
    }

    pub fn update_last_processed_block(&self, address: &Address, block_number: u64) -> Result<()> {
        self.conn.execute(
            Self::UPDATE_LAST_PROCESSED_BLOCK,
            params![block_number, format!("{:?}", address)],
        )?;
        Ok(())
    }
}
