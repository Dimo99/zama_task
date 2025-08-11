use anyhow::Result;
use clap::{Parser, Subcommand};
use eth_indexer::config::Config;
use eth_indexer::query::commands::{
    AddressHistoryQuery, TransferQuery, cmd_address_history, cmd_balance, cmd_stats,
    cmd_top_holders, cmd_transfers,
};
use eth_indexer::query::formatters::OutputFormat;
use eth_indexer::repository::{BalanceRepository, Database, TokenRepository, TransferRepository};

#[derive(Parser)]
#[command(name = "query")]
#[command(about = "Query indexed ERC20 transfer data", long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "table")]
    format: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Balance {
        address: String,
    },
    Transfers {
        #[arg(long)]
        from: Option<String>,

        #[arg(long)]
        to: Option<String>,

        #[arg(long)]
        block: Option<u64>,

        #[arg(long, num_args = 2, value_names = ["START", "END"])]
        block_range: Option<Vec<u64>>,

        #[arg(long, default_value = "false")]
        finalized: bool,

        #[arg(long, default_value = "100")]
        limit: usize,

        #[arg(long, default_value = "0")]
        offset: usize,
    },
    TopHolders {
        #[arg(default_value = "10")]
        count: usize,
    },
    Stats,
    AddressHistory {
        address: String,
        #[arg(long, default_value = "false")]
        finalized: bool,
        #[arg(long, default_value = "100")]
        limit: usize,
        #[arg(long, default_value = "0")]
        offset: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let format = OutputFormat::from(cli.format.as_str());

    let config = Config::from_env()?;

    let db = Database::new(&config.database_url)?;
    let transfer_repo = TransferRepository::new(&db.conn);
    let token_repo = TokenRepository::new(&db.conn);
    let balance_repo = BalanceRepository::new(&db.conn);
    let token_address = &config.erc20_contract_address;

    match cli.command {
        Commands::Balance { address } => {
            cmd_balance(&balance_repo, &token_repo, token_address, &address, &format)?;
        }
        Commands::Transfers {
            from,
            to,
            block,
            block_range,
            finalized,
            limit,
            offset,
        } => {
            let range = block_range.map(|v| if v.len() >= 2 { (v[0], v[1]) } else { (0, 0) });
            let query = TransferQuery {
                from,
                to,
                block,
                block_range: range,
                finalized,
                limit,
                offset,
            };
            cmd_transfers(&transfer_repo, &token_repo, token_address, query, &format)?;
        }
        Commands::TopHolders { count } => {
            cmd_top_holders(&balance_repo, &token_repo, token_address, count, &format)?;
        }
        Commands::Stats => {
            cmd_stats(&transfer_repo, &format)?;
        }
        Commands::AddressHistory {
            address,
            finalized,
            limit,
            offset,
        } => {
            let query = AddressHistoryQuery {
                address,
                finalized,
                limit,
                offset,
            };
            cmd_address_history(&transfer_repo, &token_repo, token_address, query, &format)?;
        }
    }

    Ok(())
}
