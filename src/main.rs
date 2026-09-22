mod borsh; mod events; mod tx; mod stocks; mod metrics;
mod push;
mod store; mod stream; mod enrich; mod backfill;
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
    /// Run the live indexer: Kaldera stream -> decode -> Postgres -> ape-be push
    Stream,
    /// Run one enrichment pass (supply, metadata, image) for tokens missing them
    Enrich,
    /// Print the Metaplex metadata PDA for a mint (self-check of the derivation)
    Pda { mint: String },
    /// Backfill the catalog from Raydium, pump.fun and DexScreener, then candle history
    Backfill { #[arg(long, default_value_t = 200)] ray_pages: usize, #[arg(long, default_value_t = 40)] pump_pages: usize, #[arg(long, default_value_t = 400)] candle_tokens: i64 },
    /// Candle history only, for tokens that have none yet
    Candles { #[arg(long, default_value_t = 600)] tokens: i64, #[arg(long, default_value_t = 1500)] max_per_pool: usize },
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv(); // optional: in Docker the env comes from compose
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
            let n = stocks::refresh_prices(&db, false).await?;
            println!("stocks={} priced={}", mints.len(), n);
            if watch { stocks::price_loop(db).await; }
        }
        Cmd::Stream => {
            metrics::install();
            if let Ok(u) = std::env::var("RPC_URL") { tokio::spawn(metrics::rpc_slot_loop(u)); }
            let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL")?).await?;
            stocks::sync_list(&db).await?;
            stocks::refresh_prices(&db, false).await?;
            tokio::spawn(stocks::price_loop(db.clone()));
            tokio::spawn(enrich::run(db.clone()));
            tokio::spawn(enrich::dex_paid_loop(db.clone()));
            let push = match (std::env::var("INGEST_URL"), std::env::var("INGEST_SECRET")) { (Ok(u), Ok(k)) => Some((u, k)), _ => { tracing::warn!("INGEST_URL/INGEST_SECRET unset: live push disabled"); None } };
            tokio::spawn(store::rollup_loop(db.clone(), push.clone().map(|(u, k)| push::Pusher::start(u, k))));
            tokio::spawn(stocks::tick_loop(db.clone(), push.clone().map(|(u, k)| push::Pusher::start(u, k))));
            let store = store::Store::open(db, push).await?;
            stream::run(store).await?;
        }
        Cmd::Enrich => {
            let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL")?).await?;
            let n = enrich::pass(&db, &enrich::Rpc::new(), &reqwest::Client::new(), 200).await?;
            println!("enriched {n}");
        }
        Cmd::Pda { mint } => println!("{}", enrich::metadata_pda(&mint)?),
        Cmd::Backfill { ray_pages, pump_pages, candle_tokens } => {
            let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL")?).await?;
            stocks::sync_list(&db).await?;
            let a = backfill::raydium(&db, ray_pages).await.unwrap_or_else(|e| { tracing::error!(%e, "raydium"); 0 });
            let b = backfill::pump(&db, pump_pages).await.unwrap_or_else(|e| { tracing::error!(%e, "pump"); 0 });
            let c = backfill::dbc(&db).await.unwrap_or_else(|e| { tracing::error!(%e, "dbc"); 0 });
            let d = backfill::candles(&db, candle_tokens, 3000).await.unwrap_or_else(|e| { tracing::error!(%e, "candles"); 0 });
            println!("raydium={a} pump={b} dbc={c} candles_for={d}");
        }
        Cmd::Candles { tokens, max_per_pool } => {
            let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL")?).await?;
            let d = backfill::candles(&db, tokens, max_per_pool).await?;
            println!("candles_for={d}");
        }
    }
    Ok(())
}
