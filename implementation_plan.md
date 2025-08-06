# Ethereum Log Indexer Implementation Plan

## Overview
Build a Rust service that indexes ERC-20 Transfer events from Ethereum via JSON-RPC and stores them in SQLite.

## Architecture Decisions

### Technology Stack
- **Language**: Rust with tokio async runtime
- **Ethereum Client**: Alloy (modern Rust Ethereum library)
- **Database**: SQLite with rusqlite
- **Configuration**: .env file with dotenv crate
- **Logging**: tracing with tracing-subscriber

### Database Schema
```sql
-- Tokens table
CREATE TABLE tokens (
    address TEXT PRIMARY KEY,
    deployment_block INTEGER NOT NULL
);

-- Transfers table
CREATE TABLE transfers (
    transaction_hash TEXT NOT NULL,
    log_index INTEGER NOT NULL,
    token_address TEXT NOT NULL,
    from_address TEXT NOT NULL,
    to_address TEXT NOT NULL,
    value TEXT NOT NULL, -- String to handle U256
    block_number INTEGER NOT NULL,
    PRIMARY KEY (transaction_hash, log_index),
    FOREIGN KEY (token_address) REFERENCES tokens(address)
);

-- Indexer state table
CREATE TABLE indexer_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    last_processed_block INTEGER NOT NULL
);
```

## Implementation Steps

### Phase 1: Project Setup
1. **Initialize Rust project**
   ```bash
   cargo new eth-indexer
   cd eth-indexer
   ```

2. **Add dependencies to Cargo.toml**
   ```toml
   [dependencies]
   tokio = { version = "1.0", features = ["full"] }
   alloy = { version = "0.4", features = ["provider-http", "rpc-types", "serde"] }
   rusqlite = { version = "0.32", features = ["bundled"] }
   serde = { version = "1.0", features = ["derive"] }
   dotenv = "0.15"
   anyhow = "1.0"
   tracing = "0.1"
   tracing-subscriber = "0.3"
   ```

3. **Create .env file**
   ```env
   JSON_RPC_URL=https://eth-mainnet.alchemyapi.io/v2/YOUR_KEY
   ERC20_CONTRACT_ADDRESS=0xA0b86991c6431c2c2b6c3b5cb4c5f64d1e3b7a3
   DATABASE_URL=sqlite:./indexer.db
   ```

### Phase 2: Core Components

#### 2.1 Configuration Module (`src/config.rs`)
- Load environment variables
- Parse and validate addresses
- Return structured Config struct

#### 2.2 Database Module (`src/db.rs`)
- Initialize SQLite connection
- Create tables if not exist
- Implement methods:
  - `insert_token(address, deployment_block)`
  - `insert_transfer(transfer_data)`
  - `get_last_processed_block()`
  - `update_last_processed_block(block_number)`

#### 2.3 RPC Client Module (`src/rpc.rs`)
- Initialize Alloy provider
- Implement methods:
  - `get_latest_block()`
  - `get_code_at_block(address, block)` - for deployment detection
  - `get_logs(from_block, to_block, address, topics)`

#### 2.4 Event Decoder Module (`src/events.rs`)
- Define Transfer event structure
- Parse log data into Transfer struct
- Handle value as U256 -> String conversion

### Phase 3: Core Logic

#### 3.1 Contract Deployment Finder (`src/deployment.rs`)
Binary search implementation:
```rust
async fn find_deployment_block(provider, address, latest_block) -> u64 {
    let mut left = 0;
    let mut right = latest_block;
    
    while left < right {
        let mid = (left + right) / 2;
        let code = get_code_at_block(address, mid).await;
        
        if code.is_empty() {
            left = mid + 1;
        } else {
            right = mid;
        }
    }
    
    return left;
}
```

#### 3.2 Block Scanner (`src/scanner.rs`)
- Fetch logs in batches (e.g., 1000 blocks at a time)
- Filter for Transfer event topic: `0xddf252ad...`
- Process and store each transfer
- Update last_processed_block after each batch

### Phase 4: Main Application (`src/main.rs`)
1. Load configuration
2. Initialize database
3. Check if token exists in DB:
   - If not, find deployment block and insert
4. Get last_processed_block or use deployment_block
5. Start scanning loop:
   - Fetch current block
   - Process blocks in batches until caught up
   - Store transfers with deduplication
   - Log progress
6. Switch to polling mode when caught up:
   - Check for new blocks every 12 seconds
   - Process any new blocks immediately
   - Continue polling indefinitely

### Phase 5: Error Handling & Resilience
- Retry logic for RPC calls
- Database transaction handling
- Graceful shutdown on SIGTERM
- Resume from last checkpoint on restart

### Phase 6: Testing
- Unit tests for each module
- Integration test with local Anvil node
- Test duplicate handling
- Test recovery from interruption

### Phase 7: Bonus Features
1. **Reorg Handling**
   - Store block_hash in transfers
   - Check if blocks changed on restart
   - Rollback and re-index if needed

2. **CLI Query Tool** (separate binary)
   ```bash
   cargo run --bin query -- --from 0x123... --limit 100
   cargo run --bin query -- --block 18500000
   cargo run --bin query -- --stats
   ```

## File Structure
```
eth-indexer/
├── Cargo.toml
├── .env
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── db.rs
│   ├── rpc.rs
│   ├── events.rs
│   ├── deployment.rs
│   ├── scanner.rs
│   └── bin/
│       └── query.rs
└── tests/
    └── integration_test.rs
```

## Success Criteria
- [x] Connects to Ethereum RPC
- [x] Finds contract deployment block
- [x] Scans and stores Transfer events
- [x] Handles deduplication via composite PK
- [x] Resumes from last processed block
- [ ] Handles reorgs (bonus)
- [ ] Provides CLI query tool (bonus)