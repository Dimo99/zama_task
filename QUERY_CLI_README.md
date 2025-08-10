# ERC20 Transfer Query CLI

A command-line interface tool for querying indexed ERC20 transfer data from the SQLite database.

## Installation

Build the query tool:
```bash
cargo build --release --bin query
```

The binary will be available at `./target/release/query`

## Usage

### Global Options

- `-f, --format <FORMAT>` - Output format: `table` (default), `json`, or `csv`

### Commands

#### 1. Get Balance
Get the current balance of an address (calculated from all transfers):

```bash
# Table format (default)
./target/release/query balance 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb1

# JSON format
./target/release/query -f json balance 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb1
```

#### 2. Query Transfers
Query transfers with various filters:

```bash
# Get transfers from a specific address
./target/release/query transfers --from 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb1

# Get transfers to a specific address
./target/release/query transfers --to 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb1

# Get transfers in a specific block
./target/release/query transfers --block 1000000

# Get transfers in a block range
./target/release/query transfers --block-range 1000000 1001000

# Combine with pagination
./target/release/query transfers --from 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb1 --limit 50 --offset 100

# Export to CSV
./target/release/query -f csv transfers --from 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb1 > transfers.csv
```

#### 3. Top Token Holders
Get the top N token holders by balance:

```bash
# Get top 10 holders (default)
./target/release/query top-holders

# Get top 20 holders
./target/release/query top-holders 20

# Export to JSON
./target/release/query -f json top-holders 10 > top_holders.json
```

#### 4. Database Statistics
Show overall statistics of the indexed data:

```bash
./target/release/query stats
```

Output includes:
- Total number of transfers
- Number of unique addresses
- Earliest block number
- Latest block number

#### 5. Address History
Get complete transfer history for an address (both sent and received):

```bash
# Get all transfers involving an address
./target/release/query address-history 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb1

# Export to CSV for analysis
./target/release/query -f csv address-history 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb1 > address_history.csv
```

## Output Formats

### Table Format (Default)
Human-readable ASCII tables with proper formatting:
```
╭───────────┬──────────────┬──────────────┬─────────────┬─────────────╮
│ Block     │ From         │ To           │ Value       │ Tx Hash     │
├───────────┼──────────────┼──────────────┼─────────────┼─────────────┤
│ 15234567  │ 0x123...abc  │ 0x456...def  │ 1000000000  │ 0x789...xyz │
╰───────────┴──────────────┴──────────────┴─────────────┴─────────────╯
```

### JSON Format
Structured JSON output for programmatic use:
```json
[
  {
    "block_number": 15234567,
    "from": "0x123...",
    "to": "0x456...",
    "value": "1000000000",
    "transaction_hash": "0x789...",
    "log_index": 42
  }
]
```

### CSV Format
Standard CSV format for spreadsheet import:
```csv
block_number,from,to,value,transaction_hash,log_index
15234567,0x123...,0x456...,1000000000,0x789...,42
```

## Examples

### Analyze Token Distribution
```bash
# Get top 100 holders and save to CSV
./target/release/query -f csv top-holders 100 > distribution.csv

# Get all transfers for analysis
./target/release/query -f csv transfers --block-range 0 999999999 --limit 1000000 > all_transfers.csv
```

### Track Address Activity
```bash
# Check balance
./target/release/query balance 0xYourAddress

# Get recent sends
./target/release/query transfers --from 0xYourAddress --limit 10

# Get recent receives
./target/release/query transfers --to 0xYourAddress --limit 10

# Full history
./target/release/query address-history 0xYourAddress
```

### Monitor Recent Activity
```bash
# Get latest transfers (assuming latest block is around 18000000)
./target/release/query transfers --block-range 17999000 18000000
```

## Performance Notes

- The database has indexes on `from_address`, `to_address`, and `block_number` for fast queries
- Use pagination (`--limit` and `--offset`) for large result sets
- The `top-holders` query may be slow for large databases as it needs to aggregate all transfers

## Troubleshooting

### Database Not Found
If you get a database error, make sure:
1. The indexer has run and created the database
2. You're specifying the correct path with `-d` flag
3. The database file has read permissions

### Invalid Address Format
Addresses must be in hexadecimal format starting with `0x`:
- ✅ Correct: `0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb1`
- ❌ Wrong: `742d35Cc6634C0532925a3b844Bc9e7595f0bEb1` (missing 0x)

### Large Result Sets
For queries returning many results:
1. Use pagination with `--limit` and `--offset`
2. Export to CSV or JSON for processing in other tools
3. Consider filtering by block range to reduce results