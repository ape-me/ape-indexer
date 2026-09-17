mod borsh; mod events; mod tx; mod stocks; mod store; mod stream; mod enrich;
#[macro_use] mod decode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::HashSet;

#[derive(Parser)]
#[command(name = "ape-indexer")]
struct Cli { #[command(subcommand)] cmd: Cmd }

#[derive(Subcommand)]
enum Cmd {
    /// Decode RPC-json transactions from files and print events as JSON lines
    DecodeFiles { #[arg(long)] stocks: String, files: Vec<String> },
    /// Sync the stock list from StonkFun and refresh USD prices once (or loop with --watch)
    Stocks { #[arg(long)] watch: bool },
    /// Run the live indexer: Kaldera stream -> decode -> Postgres -> Redis
    Stream,
    /// Run one enrichment pass (supply, metadata, image) for tokens missing them
    Enrich,
    /// Print the Metaplex metadata PDA for a mint (self-check of the derivation)
    Pda { mint: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::from_path("/root/ape-indexer/.env");
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    match Cli::parse().cmd {
        Cmd::DecodeFiles { stocks, files } => {
            let stocks: HashSet<String> = std::fs::read_to_string(stocks)?.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
            for f in files {
                let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f)?)?;
                let view = tx::TxView::from_rpc_json(&v)?;
                for ev in decode::decode(&view, &stocks) { println!("{}", serde_json::to_string(&ev)?); }
            }
        }
        Cmd::Stocks { watch } => {
            let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL")?).await?;
            let mints = stocks::sync_list(&db).await?;
            let n = stocks::refresh_prices(&db).await?;
            println!("stocks={} priced={}", mints.len(), n);
            if watch { stocks::price_loop(db).await; }
        }
        Cmd::Stream => {
            let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL")?).await?;
            stocks::sync_list(&db).await?;
            stocks::refresh_prices(&db).await?;
            tokio::spawn(stocks::price_loop(db.clone()));
            tokio::spawn(enrich::run(db.clone()));
            let store = store::Store::open(db, std::env::var("REDIS_URL").ok().as_deref()).await?;
            stream::run(store).await?;
        }
        Cmd::Enrich => {
            let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL")?).await?;
            let n = enrich::pass(&db, &enrich::Rpc::new(), &reqwest::Client::new(), 200).await?;
            println!("enriched {n}");
        }
        Cmd::Pda { mint } => println!("{}", enrich::metadata_pda(&mint)?),
    }
    Ok(())
}
