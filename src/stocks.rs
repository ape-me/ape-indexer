//! Stock dictionary: 85 real stock mints from StonkFun's quote list, USD prices from DexScreener every 30s.
use anyhow::{Context, Result};
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::HashSet;
use std::time::Duration;

const STONKFUN: &str = "https://www.stonkfun.xyz/api/quote-tokens";
const DEXSCREENER: &str = "https://api.dexscreener.com/tokens/v1/solana/";
const STOCK_CATEGORIES: [&str; 3] = ["xstock", "backpack", "prestock"];
/// Non-PreStocks pre-IPO mints. Never indexed: PreStocks bounty disqualifies any project that integrates them.
const EXCLUDED_MINTS: [&str; 1] = [
    "Xs3oZwbHvqis4NYcf4YKWmEia2eC84wSiVrcYcTqpH8", // SPCXx (xStocks SpaceX)
];

#[derive(Deserialize)]
struct QuoteToken { #[serde(rename = "quoteMint")] mint: String, symbol: String, name: String, decimals: i16, #[serde(rename = "logoUrl")] logo: Option<String>, category: String }
#[derive(Deserialize)]
struct QuoteList { #[serde(rename = "quoteTokens")] tokens: Vec<QuoteToken> }

fn client() -> reqwest::Client {
    reqwest::Client::builder().user_agent("ape-indexer/0.1").timeout(Duration::from_secs(15)).build().unwrap()
}

fn issuer(cat: &str) -> &'static str { match cat { "xstock" => "xstocks", "backpack" => "backpack", "prestock" => "prestocks", _ => "other" } }
/// Crypto pairs Backpack lists next to equities. They keep their floors but are not counted as stocks.
const CRYPTO: [&str; 12] = ["ARB", "CHIP", "DOGE", "INJ", "LINK", "PEPE", "TAO", "PEAQ", "PONS", "ROBOSTRATEGY", "PSG", "PENG"];
fn category(cat: &str, symbol: &str) -> &'static str {
    if CRYPTO.contains(&symbol) { return "crypto" }
    match cat { "prestock" => "preipo", _ => if symbol.ends_with('X') && ["SPY", "QQQ", "TQQQ", "GLD", "VTI", "IWM", "DIA"].iter().any(|e| symbol.starts_with(e)) { "etf" } else { "stock" } }
}

/// Upsert the stock list. Returns the set of stock mints.
pub async fn sync_list(db: &PgPool) -> Result<HashSet<String>> {
    let list: QuoteList = client().get(STONKFUN).send().await?.error_for_status()?.json().await.context("stonkfun quote-tokens")?;
    let now = chrono_now();
    let mut mints = HashSet::new();
    for t in list.tokens.into_iter().filter(|t| STOCK_CATEGORIES.contains(&t.category.as_str()) && !EXCLUDED_MINTS.contains(&t.mint.as_str())) {
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

const PRESTOCKS: &str = "https://prestocks.com/api/prestocks";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreStock { #[serde(rename = "contract_address")] contract_address: String, mark_price: Option<f64>, mark_valuation: Option<f64>, implied_valuation: Option<f64>, supply: Option<f64> }

/// PreStocks' own numbers per mint: mark price of the underlying, valuations, supply. Empty map on any failure.
async fn prestocks_marks(c: &reqwest::Client) -> std::collections::HashMap<String, PreStock> {
    match c.get(PRESTOCKS).send().await.and_then(|r| r.error_for_status()) {
        Ok(r) => r.json::<Vec<PreStock>>().await.unwrap_or_default().into_iter().map(|p| (p.contract_address.clone(), p)).collect(),
        Err(e) => { tracing::warn!(%e, "prestocks api"); Default::default() }
    }
}

/// One pass of stock market data: 30 mints per DexScreener call, best pair by liquidity, plus PreStocks marks.
/// Updates the live columns on `stocks`; with `snapshot` also appends a row per stock to `stock_snapshots`.
pub async fn refresh_prices(db: &PgPool, snapshot: bool) -> Result<usize> {
    let mints: Vec<String> = sqlx::query_scalar("SELECT mint FROM stocks").fetch_all(db).await?;
    let c = client(); let now = chrono_now(); let mut updated = 0;
    let marks = prestocks_marks(&c).await;
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
            let f = |v: &serde_json::Value| v.as_f64();
            let i = |v: &serde_json::Value| v.as_i64().map(|x| x as i32);
            let (liq, v24, v1, b24, s24, mcap) = (f(&p["liquidity"]["usd"]), f(&p["volume"]["h24"]), f(&p["volume"]["h1"]), i(&p["txns"]["h24"]["buys"]), i(&p["txns"]["h24"]["sells"]), f(&p["marketCap"]).or(f(&p["fdv"])));
            let (change, change_1h) = (f(&p["priceChange"]["h24"]), f(&p["priceChange"]["h1"]));
            let m = marks.get(mint.as_str());
            let mark = m.and_then(|m| m.mark_price);
            let premium = mark.filter(|m| *m > 0.0).map(|m| (price / m - 1.0) * 100.0);
            let (mval, ival, supply) = (m.and_then(|m| m.mark_valuation), m.and_then(|m| m.implied_valuation), m.and_then(|m| m.supply));
            sqlx::query("UPDATE stocks SET price_usd=$2, change_24h=$3, updated_at=$4, liquidity_usd=$5, vol_24h_usd=$6, vol_1h_usd=$7, buys_24h=$8, sells_24h=$9, mcap_usd=$10, change_1h=$11, mark_usd=$12, premium_pct=$13, mark_valuation=$14, implied_valuation=$15, supply=$16 WHERE mint=$1")
                .bind(mint).bind(price).bind(change).bind(now).bind(liq).bind(v24).bind(v1).bind(b24).bind(s24).bind(mcap).bind(change_1h).bind(mark).bind(premium).bind(mval).bind(ival).bind(supply)
                .execute(db).await?;
            if snapshot {
                sqlx::query("INSERT INTO stock_snapshots (mint, ts, price_usd, liquidity_usd, vol_24h_usd, vol_1h_usd, buys_24h, sells_24h, mcap_usd, mark_usd, premium_pct, mark_valuation, implied_valuation, supply) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) ON CONFLICT DO NOTHING")
                    .bind(mint).bind(now).bind(price).bind(liq).bind(v24).bind(v1).bind(b24).bind(s24).bind(mcap).bind(mark).bind(premium).bind(mval).bind(ival).bind(supply)
                    .execute(db).await?;
            }
            updated += 1;
        }
        tokio::time::sleep(Duration::from_millis(250)).await; // 4 calls/s max, far under 300/min
    }
    Ok(updated)
}

/// Every 30s: live columns. Every other pass (60s): a snapshot row. Once an hour: drop snapshots older than 30 days.
pub async fn price_loop(db: PgPool) {
    let mut pass: u64 = 0;
    loop {
        let last = std::time::Instant::now();
        match refresh_prices(&db, pass % 2 == 0).await { Ok(n) => { crate::metrics::stock_prices_refreshed(); tracing::info!(n, "stock prices refreshed") }, Err(e) => tracing::warn!(%e, "price loop") }
        if pass % 120 == 0 {
            let cutoff = chrono_now() - 30 * 86400;
            if let Err(e) = sqlx::query("DELETE FROM stock_snapshots WHERE ts < $1").bind(cutoff).execute(&db).await { tracing::warn!(%e, "snapshot retention") }
        }
        pass += 1;
        for _ in 0..30 { tokio::time::sleep(Duration::from_secs(1)).await; crate::metrics::stock_prices_age(last.elapsed().as_secs_f64()); }
    }
}

pub fn chrono_now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 }
