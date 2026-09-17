mod borsh; mod events; mod tx; mod stocks;
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
    }
    Ok(())
}
