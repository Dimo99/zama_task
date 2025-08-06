use anyhow::{Context, Result};
use rusqlite::Connection;

pub struct Database {
    pub conn: Connection,
}

impl Database {
    pub fn new(db_path: &str) -> Result<Self> {
        let db_path = db_path.strip_prefix("sqlite:").unwrap_or(db_path);
        let conn = Connection::open(db_path)
            .context("Failed to open database")?;
        
        let db = Database { conn };
        db.create_tables()?;
        Ok(db)
    }

    fn create_tables(&self) -> Result<()> {
        // Create tokens table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS tokens (
                address TEXT PRIMARY KEY,
                deployment_block INTEGER NOT NULL,
                last_processed_block INTEGER
            )",
            [],
        )?;

        // Create transfers table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS transfers (
                transaction_hash TEXT NOT NULL,
                log_index INTEGER NOT NULL,
                token_address TEXT NOT NULL,
                from_address TEXT NOT NULL,
                to_address TEXT NOT NULL,
                value TEXT NOT NULL,
                block_number INTEGER NOT NULL,
                PRIMARY KEY (transaction_hash, log_index),
                FOREIGN KEY (token_address) REFERENCES tokens(address)
            )",
            [],
        )?;

        // Create indexes for better query performance
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_transfers_block_number 
             ON transfers(block_number)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_transfers_from 
             ON transfers(from_address)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_transfers_to 
             ON transfers(to_address)",
            [],
        )?;

        Ok(())
    }
}