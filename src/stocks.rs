//! Stock dictionary: 85 real stock mints from StonkFun's quote list, USD prices from DexScreener every 30s.
use anyhow::{Context, Result};
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const STONKFUN: &str = "https://www.stonkfun.xyz/api/quote-tokens";
const DEXSCREENER: &str = "https://api.dexscreener.com/tokens/v1/solana/";
const STOCK_CATEGORIES: [&str; 3] = ["xstock", "backpack", "prestock"];
/// Non-PreStocks pre-IPO mints. Never indexed: PreStocks bounty disqualifies any project that integrates them.
// Exclusions live in stock_config (see migration 0009), not here: flipping a row needs no deploy.

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
    // Operator overrides: excluded stocks are kept in the table (flagged) but never subscribed to.
    let cfg: HashMap<String, (bool, Option<String>, Vec<String>)> = sqlx::query_as::<_, (String, bool, Option<String>, Vec<String>)>("SELECT mint, excluded, category, tags FROM stock_config")
        .fetch_all(db).await?.into_iter().map(|r| (r.0, (r.1, r.2, r.3))).collect();
    for t in list.tokens.into_iter().filter(|t| STOCK_CATEGORIES.contains(&t.category.as_str())) {
        let logo = t.logo.map(|l| if l.starts_with('/') { format!("https://www.stonkfun.xyz{l}") } else { l });
        let (excluded, cat_override, tags) = cfg.get(&t.mint).cloned().unwrap_or((false, None, vec![]));
        let cat = cat_override.unwrap_or_else(|| category(&t.category, &t.symbol).to_string());
        sqlx::query("INSERT INTO stocks (mint, symbol, name, issuer, category, decimals, logo, updated_at, excluded, tags) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
                     ON CONFLICT (mint) DO UPDATE SET symbol=EXCLUDED.symbol, name=EXCLUDED.name, issuer=EXCLUDED.issuer, category=EXCLUDED.category, decimals=EXCLUDED.decimals, logo=EXCLUDED.logo, excluded=EXCLUDED.excluded, tags=EXCLUDED.tags")
            .bind(&t.mint).bind(&t.symbol).bind(&t.name).bind(issuer(&t.category)).bind(&cat).bind(t.decimals).bind(logo).bind(now).bind(excluded).bind(&tags)
            .execute(db).await?;
        if !excluded { mints.insert(t.mint); }
    }
    tracing::info!(n = mints.len(), "stocks synced");
    if let Err(e) = refresh_multipliers(db).await { tracing::warn!(%e, "multipliers") }
    Ok(mints)
}

/// Token-2022 scaled-UI multiplier per stock. xStocks/Backpack pay dividends and do splits by raising it
/// (1 raw unit = multiplier displayed units), so every raw→USD conversion must include it. A pending
/// `newMultiplier` takes over once its effective time has passed. Mints without the extension stay at 1.
pub async fn refresh_multipliers(db: &PgPool) -> Result<usize> {
    let rpc = crate::enrich::Rpc::new();
    let mints: Vec<String> = sqlx::query_scalar("SELECT mint FROM stocks").fetch_all(db).await?;
    let now = chrono_now(); let mut n = 0;
    for mint in mints {
        let r = match rpc.call("getAccountInfo", serde_json::json!([mint, {"encoding":"jsonParsed"}])).await { Ok(r) => r, Err(e) => { tracing::warn!(%e, mint, "multiplier"); continue } };
        let mut m = 1.0;
        for ext in r["value"]["data"]["parsed"]["info"]["extensions"].as_array().cloned().unwrap_or_default() {
            if ext["extension"].as_str() != Some("scaledUiAmountConfig") { continue }
            let st = &ext["state"];
            let cur = st["multiplier"].as_str().and_then(|x| x.parse::<f64>().ok()).or_else(|| st["multiplier"].as_f64()).unwrap_or(1.0);
            let new = st["newMultiplier"].as_str().and_then(|x| x.parse::<f64>().ok()).or_else(|| st["newMultiplier"].as_f64());
            let eff = st["newMultiplierEffectiveTimestamp"].as_i64().unwrap_or(i64::MAX);
            m = match new { Some(nm) if eff <= now && nm > 0.0 => nm, _ => cur };
        }
        if m > 0.0 { sqlx::query("UPDATE stocks SET multiplier = $2 WHERE mint = $1 AND multiplier IS DISTINCT FROM $2").bind(&mint).bind(m).execute(db).await?; n += 1; }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    tracing::info!(n, "multipliers refreshed");
    Ok(n)
}

const PRESTOCKS: &str = "https://prestocks.com/api/prestocks";
const JUPITER_PRICE: &str = "https://lite-api.jup.ag/price/v3?ids=";

/// Jupiter's aggregated price per mint: routes across every pool, so it is what a buyer actually pays.
/// DexScreener only sees the pools it lists (OPENAI was +48% off). `stockData` carries the underlying stock's
/// real price for tokenized equities, which gives premium-to-underlying for xStocks/Backpack too.
struct JupPrice { usd: f64, liquidity: Option<f64>, underlying: Option<f64>, underlying_mcap: Option<f64> }
async fn jupiter_prices(c: &reqwest::Client, mints: &[String]) -> std::collections::HashMap<String, JupPrice> {
    let mut out = std::collections::HashMap::new();
    for chunk in mints.chunks(50) {
        let url = format!("{JUPITER_PRICE}{}", chunk.join(","));
        let v: serde_json::Value = match c.get(&url).send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(e) => { tracing::warn!(%e, "jupiter price"); continue }
        };
        if let Some(obj) = v.as_object() {
            for (mint, j) in obj {
                let Some(usd) = j["usdPrice"].as_f64() else { continue };
                out.insert(mint.clone(), JupPrice { usd, liquidity: j["liquidity"].as_f64(), underlying: j["stockData"]["price"].as_f64(), underlying_mcap: j["stockData"]["mcap"].as_f64() });
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    out
}

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
    let jup = jupiter_prices(&c, &mints).await;
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
            let empty = serde_json::Value::Null;
            let p = best.unwrap_or(&empty);
            let f = |v: &serde_json::Value| v.as_f64();
            let i = |v: &serde_json::Value| v.as_i64().map(|x| x as i32);
            let j = jup.get(mint.as_str());
            // price: Jupiter first (all pools), DexScreener's best pool as fallback
            let Some(price) = j.map(|j| j.usd).or_else(|| p["priceUsd"].as_str().and_then(|s| s.parse::<f64>().ok())) else { continue };
            let liq = match (j.and_then(|j| j.liquidity), f(&p["liquidity"]["usd"])) { (Some(a), Some(b)) => Some(a.max(b)), (a, b) => a.or(b) };
            let (v24, v1, b24, s24, mcap) = (f(&p["volume"]["h24"]), f(&p["volume"]["h1"]), i(&p["txns"]["h24"]["buys"]), i(&p["txns"]["h24"]["sells"]), f(&p["marketCap"]).or(f(&p["fdv"])));
            let (change, change_1h) = (f(&p["priceChange"]["h24"]), f(&p["priceChange"]["h1"]));
            let m = marks.get(mint.as_str());
            // mark = PreStocks' published mark for pre-IPO, else the real underlying stock price from Jupiter
            let mark = m.and_then(|m| m.mark_price).or_else(|| j.and_then(|j| j.underlying));
            let premium = mark.filter(|m| *m > 0.0).map(|m| (price / m - 1.0) * 100.0);
            let (mval, ival, supply) = (m.and_then(|m| m.mark_valuation).or_else(|| j.and_then(|j| j.underlying_mcap)), m.and_then(|m| m.implied_valuation), m.and_then(|m| m.supply));
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

/// Every 5s: Jupiter price for every stock → stock_ticks (24h retention) and a `price` frame to the stock's room.
/// Only pushes when the price moved, so a quiet stock costs nothing. Feeds /history 5m/15m/1h and the live hero price.
pub async fn tick_loop(db: PgPool, push: Option<crate::push::Pusher>) {
    let c = client(); let mut last: HashMap<String, f64> = HashMap::new(); let mut pass: u64 = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let rows: Vec<(String, Option<f64>, Option<f64>)> = match sqlx::query_as("SELECT mint, mark_usd, change_24h FROM stocks WHERE NOT excluded").fetch_all(&db).await { Ok(r) => r, Err(e) => { tracing::warn!(%e, "ticks: stocks"); continue } };
        let mints: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let jup = jupiter_prices(&c, &mints).await;
        if jup.is_empty() { continue }
        let now = chrono_now();
        for (mint, mark, chg) in &rows {
            let Some(j) = jup.get(mint) else { continue };
            if let Err(e) = sqlx::query("INSERT INTO stock_ticks (mint, ts, price_usd) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING").bind(mint).bind(now).bind(j.usd).execute(&db).await { tracing::warn!(%e, "ticks: insert"); }
            if last.get(mint).map_or(true, |p| (p - j.usd).abs() > f64::EPSILON) {
                last.insert(mint.clone(), j.usd);
                if let Some(p) = &push { p.send_price(crate::push::IngestPrice { mint: mint.clone(), ts: now, price_usd: j.usd, mark_usd: *mark, change_24h: *chg }); }
            }
        }
        pass += 1;
        if pass % 720 == 0 { if let Err(e) = sqlx::query("DELETE FROM stock_ticks WHERE ts < $1").bind(now - 86400).execute(&db).await { tracing::warn!(%e, "ticks: retention") } }
    }
}

pub fn chrono_now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 }
