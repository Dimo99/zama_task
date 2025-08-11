# ERC20 Transfer Indexer

A high-performance Ethereum ERC20 transfer indexer that stores transfer data in SQLite with automatic finality tracking and reorg detection.

## Features

- **Parallel RPC fetching** with configurable batch sizes and rate limiting
- **Automatic finality tracking** to mark transfers as confirmed after 2 epochs
- **Chain reorganization detection** and automatic correction
- **Denormalized balance table** for instant balance queries on large tokens
- **Configurable multi-RPC support** for load distribution and failover
- **Resumable indexing** from the last processed block

## Installation

Build the indexer:
```bash
cargo build --release --bin indexer
```

The binary will be available at `./target/release/indexer`

## Configuration

The indexer is configured through environment variables. Create a `.env` file:

```env
# Required: ERC20 token contract address to index
ERC20_CONTRACT_ADDRESS=0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48

# Required: Database URL (SQLite)
DATABASE_URL=sqlite:transfers.db

# Required: Ethereum RPC endpoints (comma-separated for multiple)
JSON_RPC_URLS=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY,https://mainnet.infura.io/v3/YOUR_KEY

# Optional: Performance tuning
BATCH_SIZE=1000                    # Number of blocks per request (default: 1000)
RATE_LIMIT_DELAY_MS=500            # Delay between requests in ms (default: 500)
MAX_PENDING_REQUESTS=30            # Max concurrent RPC requests (default: 30)

# Optional: Finality settings
FINALITY_UPDATE_INTERVAL_SECS=384   # How often to check finality (default: 384)
BLOCK_TIME_SECS=12                 # Expected block time for polling (default: 12)
```

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `ERC20_CONTRACT_ADDRESS` | Yes | - | The ERC20 token contract address to index |
| `DATABASE_URL` | Yes | - | SQLite database path (prefix with `sqlite:`) |
| `JSON_RPC_URLS` | Yes | - | Comma-separated list of Ethereum RPC endpoints |
| `BATCH_SIZE` | No | 1000 | Number of blocks to fetch per RPC request |
| `RATE_LIMIT_DELAY_MS` | No | 500 | Milliseconds to wait between RPC requests |
| `MAX_PENDING_REQUESTS` | No | 30 | Maximum concurrent RPC requests |
| `FINALITY_UPDATE_INTERVAL_SECS` | No | 384 | Seconds between finality update checks (1 epoch) |
| `BLOCK_TIME_SECS` | No | 12 | Expected seconds per block for new block polling |

## Usage

### Basic Usage

Run the indexer with default settings:
```bash
./target/release/indexer
```

The indexer will:
1. Find the token's deployment block automatically
2. Start indexing from the deployment block (or resume from last processed)
3. Continue indexing until caught up with the chain head
4. Poll for new blocks when caught up

## Database Schema

The indexer creates three main tables:

### transfers
Stores all ERC20 transfer events:
- `transaction_hash` - Transaction hash
- `log_index` - Log index within transaction
- `token_address` - ERC20 token address
- `from_address` - Sender address
- `to_address` - Recipient address
- `value` - Transfer amount (as string to preserve precision)
- `block_number` - Block number
- `block_hash` - Block hash (for reorg detection)
- `is_finalized` - Whether transfer is beyond reorg possibility

### balances
Denormalized balance table for fast queries:
- `address` - Account address
- `balance_padded` - Zero-padded balance for proper sorting

### tokens
Token metadata and sync state:
- `address` - Token contract address
- `deployment_block` - Block where token was deployed
- `last_processed_block` - Latest indexed block
- `last_processed_finalized_block` - Latest finalized block
- `name` - Token name
- `symbol` - Token symbol
- `decimals` - Token decimals

## Performance Optimization

### RPC Configuration
- **Multiple RPCs**: Use multiple RPC endpoints to distribute load
- **Rate Limiting**: Adjust delay based on your RPC provider's limits
- **Concurrent Requests**: More pending requests increase throughput

## Monitoring

### Check Indexing Progress
Monitor the indexer's progress through logs:
```bash
# Follow logs
./target/release/indexer 2>&1 | tee indexer.log

# Check progress
grep "Processing" indexer.log | tail -10
grep "Caught up" indexer.log
```

## Features in Detail

### Automatic Finality Tracking
The indexer automatically tracks block finality (typically 2 epochs in Ethereum, ~12.8 minutes):
- Marks transfers as `is_finalized=true` when confirmed
- Updates denormalized balance table only for finalized transfers
- Runs finality checks every 384 seconds by default

### Chain Reorganization Detection
Automatically detects and handles chain reorganizations:
- Compares block hashes between database and chain
- Removes transfers from reorganized blocks
- Re-indexes correct transfers from the canonical chain
- Updates balances accordingly

### Resumable Indexing
The indexer automatically resumes from the last processed block:
- No need to re-index from the beginning after restarts
- Maintains consistency through database transactions
- Tracks both latest processed and latest finalized blocks

### Balance Denormalization
Maintains a denormalized balance table for instant queries:
- Updated incrementally as transfers are finalized
- Enables O(1) balance lookups instead of scanning all transfers
- Critical for tokens with millions of transfers like USDC

## Troubleshooting

### Slow Initial Sync
Speed up initial sync by:
1. Using paid RPC endpoints with higher limits
2. Adding more RPC endpoints to `JSON_RPC_URLS`
3. Decreasing `RATE_LIMIT_DELAY_MS`
4. Increasing `MAX_PENDING_REQUESTS`
5. Increasing `BATCH_SIZE` (if RPC supports it)

## Development

### Database Migrations
The indexer automatically runs migrations on startup. To run migrations manually:
```bash
./target/release/migrate
```

## Architecture

The indexer uses Tokio's async runtime with careful design for concurrent I/O:

1. **Main Scanner**: Async task that coordinates fetching and manages block ranges
2. **RPC Fetcher Tasks**: `FuturesOrdered` set - fetches logs in parallel but processes results in the order they were fired, ensuring events are always handled sequentially by block number
3. **Insertion Worker**: Async task that receives batches via channel, then uses `spawn_blocking` to run database operations on Tokio's blocking thread pool - this keeps the async runtime free while SQLite operations execute
4. **Finality Updater**: Periodic async task checking and updating transfer finality

Key design points:
- **Parallel fetching, ordered processing**: RPC requests happen concurrently but results are processed in block order
- **Non-blocking database writes**: Each batch spawns a blocking task for database operations, preventing SQLite's synchronous I/O from blocking the async runtime
- **Channel-based communication**: Async channel connects the scanner to the insertion worker
- **Sequential guarantees**: Events are always inserted in order despite parallel fetching

This architecture ensures:
- Maximum RPC throughput through parallel requests
- Correct event ordering despite concurrent fetching
- Database writes don't block the async event loop
- Pipeline processing - RPC fetching continues while previous batches are being written
