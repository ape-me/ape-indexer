//! Stock dictionary: 85 real stock mints from StonkFun's quote list, USD prices from DexScreener every 30s.
use anyhow::{Context, Result};
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::HashSet;
use std::time::Duration;

const STONKFUN: &str = "https://www.stonkfun.xyz/api/quote-tokens";
const DEXSCREENER: &str = "https://api.dexscreener.com/tokens/v1/solana/";
const STOCK_CATEGORIES: [&str; 4] = ["xstock", "backpack", "prestock", "tessera"];

#[derive(Deserialize)]
struct QuoteToken { #[serde(rename = "quoteMint")] mint: String, symbol: String, name: String, decimals: i16, #[serde(rename = "logoUrl")] logo: Option<String>, category: String }
#[derive(Deserialize)]
struct QuoteList { #[serde(rename = "quoteTokens")] tokens: Vec<QuoteToken> }

fn client() -> reqwest::Client {
    reqwest::Client::builder().user_agent("ape-indexer/0.1").timeout(Duration::from_secs(15)).build().unwrap()
}

fn issuer(cat: &str) -> &'static str { match cat { "xstock" => "xstocks", "backpack" => "backpack", "prestock" => "prestocks", "tessera" => "tessera", _ => "other" } }
fn category(cat: &str, symbol: &str) -> &'static str {
    match cat { "prestock" | "tessera" => "preipo", _ => if symbol.ends_with('X') && ["SPY", "QQQ", "TQQQ", "GLD", "VTI", "IWM", "DIA"].iter().any(|e| symbol.starts_with(e)) { "etf" } else { "stock" } }
}

/// Upsert the stock list. Returns the set of stock mints.
pub async fn sync_list(db: &PgPool) -> Result<HashSet<String>> {
    let list: QuoteList = client().get(STONKFUN).send().await?.error_for_status()?.json().await.context("stonkfun quote-tokens")?;
    let now = chrono_now();
    let mut mints = HashSet::new();
    for t in list.tokens.into_iter().filter(|t| STOCK_CATEGORIES.contains(&t.category.as_str())) {
        let logo = t.logo.map(|l| if l.starts_with('/') { format!("https://www.stonkfun.xyz{l}") } else { l });
        sqlx::query("INSERT INTO stocks (mint, symbol, name, issuer, category, decimals, logo, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                     ON CONFLICT (mint) DO UPDATE SET symbol=EXCLUDED.symbol, name=EXCLUDED.name, issuer=EXCLUDED.issuer, category=EXCLUDED.category, decimals=EXCLUDED.decimals, logo=EXCLUDED.logo")
            .bind(&t.mint).bind(&t.symbol).bind(&t.name).bind(issuer(&t.category)).bind(category(&t.category, &t.symbol)).bind(t.decimals).bind(logo).bind(now)
            .execute(db).await?;
        mints.insert(t.mint);
    }
    tracing::info!(n = mints.len(), "stocks synced");
    Ok(mints)
}

/// One pass of USD prices: 30 mints per DexScreener call, best pair by liquidity.
pub async fn refresh_prices(db: &PgPool) -> Result<usize> {
    let mints: Vec<String> = sqlx::query_scalar("SELECT mint FROM stocks").fetch_all(db).await?;
    let c = client(); let now = chrono_now(); let mut updated = 0;
    for chunk in mints.chunks(30) {
        let url = format!("{DEXSCREENER}{}", chunk.join(","));
        let pairs: Vec<serde_json::Value> = match c.get(&url).send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(e) => { tracing::warn!(%e, "dexscreener"); continue }
        };
        for mint in chunk {
            // the stock is the base token in its own pools (vs SOL/USDC); pick the most liquid one
            let best = pairs.iter().filter(|p| p["baseToken"]["address"].as_str() == Some(mint))
                .max_by(|a, b| a["liquidity"]["usd"].as_f64().unwrap_or(0.0).partial_cmp(&b["liquidity"]["usd"].as_f64().unwrap_or(0.0)).unwrap());
            let Some(p) = best else { continue };
            let Some(price) = p["priceUsd"].as_str().and_then(|s| s.parse::<f64>().ok()) else { continue };
            let change = p["priceChange"]["h24"].as_f64();
            sqlx::query("UPDATE stocks SET price_usd=$2, change_24h=$3, updated_at=$4 WHERE mint=$1").bind(mint).bind(price).bind(change).bind(now).execute(db).await?;
            updated += 1;
        }
        tokio::time::sleep(Duration::from_millis(250)).await; // 4 calls/s max, far under 300/min
    }
    Ok(updated)
}

pub async fn price_loop(db: PgPool) {
    loop {
        match refresh_prices(&db).await { Ok(n) => tracing::info!(n, "stock prices refreshed"), Err(e) => tracing::warn!(%e, "price loop") }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

pub fn chrono_now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 }
