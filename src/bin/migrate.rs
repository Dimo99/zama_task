use anyhow::Result;
use eth_indexer::repository::Database;

fn main() -> Result<()> {
    tracing_subscriber::fmt().init();
    dotenv::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL").unwrap();

    println!("Running migrations on database: {database_url}");

    let _db = Database::new(&database_url)?;

    println!("Migrations completed successfully!");

    Ok(())
}
