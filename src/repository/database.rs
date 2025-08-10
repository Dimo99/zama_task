use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::info;

pub struct Database {
    pub conn: Connection,
    db_path: String,
}

impl Database {
    pub fn new(db_path: &str) -> Result<Self> {
        let db_path = db_path.strip_prefix("sqlite:").unwrap_or(db_path);
        let conn = Connection::open(db_path).context("Failed to open database")?;

        let db = Database {
            conn,
            db_path: db_path.to_string(),
        };
        db.create_tables()?;
        Ok(db)
    }

    fn create_tables(&self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS tokens (
                address TEXT PRIMARY KEY,
                deployment_block INTEGER NOT NULL,
                last_processed_block INTEGER,
                last_processed_finalized_block INTEGER,
                name TEXT,
                symbol TEXT,
                decimals INTEGER
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS transfers (
                transaction_hash TEXT NOT NULL,
                log_index INTEGER NOT NULL,
                token_address TEXT NOT NULL,
                from_address TEXT NOT NULL,
                to_address TEXT NOT NULL,
                value TEXT NOT NULL,
                block_number INTEGER NOT NULL,
                block_hash TEXT DEFAULT '',
                is_finalized BOOLEAN DEFAULT FALSE,
                PRIMARY KEY (transaction_hash, log_index),
                FOREIGN KEY (token_address) REFERENCES tokens(address)
            )",
            [],
        )?;

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

        self.run_migrations()?;

        Ok(())
    }

    fn run_migrations(&self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        self.apply_migration(1, |conn| {
            // Migration 1: Add finality tracking columns

            let mut stmt = conn.prepare("PRAGMA table_info(transfers)")?;
            let columns: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))?
                .filter_map(Result::ok)
                .collect();

            if !columns.contains(&"block_hash".to_string()) {
                conn.execute(
                    "ALTER TABLE transfers ADD COLUMN block_hash TEXT DEFAULT ''",
                    [],
                )?;
            }

            if !columns.contains(&"is_finalized".to_string()) {
                conn.execute(
                    "ALTER TABLE transfers ADD COLUMN is_finalized BOOLEAN DEFAULT FALSE",
                    [],
                )?;

                // Mark all existing transfers as finalized (they're old data)
                conn.execute(
                    "UPDATE transfers SET is_finalized = TRUE WHERE block_hash = ''",
                    [],
                )?;
            }

            let mut stmt = conn.prepare("PRAGMA table_info(tokens)")?;
            let columns: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))?
                .filter_map(Result::ok)
                .collect();

            if !columns.contains(&"last_processed_finalized_block".to_string()) {
                conn.execute(
                    "ALTER TABLE tokens ADD COLUMN last_processed_finalized_block INTEGER",
                    [],
                )?;

                // Set last_processed_finalized_block to last_processed_block for existing data
                conn.execute(
                    "UPDATE tokens SET last_processed_finalized_block = last_processed_block 
                     WHERE last_processed_finalized_block IS NULL",
                    [],
                )?;
            }

            Ok(())
        })?;

        // Future migrations would go here:
        // self.apply_migration(2, |conn| { ... })?;

        Ok(())
    }

    fn apply_migration<F>(&self, version: i32, migration: F) -> Result<()>
    where
        F: FnOnce(&Connection) -> Result<()>,
    {
        let already_applied: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?)",
                [version],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !already_applied {
            migration(&self.conn)?;

            self.conn.execute(
                "INSERT INTO schema_migrations (version) VALUES (?)",
                [version],
            )?;

            info!("Applied migration {version}");
        }

        Ok(())
    }
}

impl Clone for Database {
    fn clone(&self) -> Self {
        let conn = Connection::open(&self.db_path)
            .expect("Failed to open database connection during clone");
        Database {
            conn,
            db_path: self.db_path.clone(),
        }
    }
}
