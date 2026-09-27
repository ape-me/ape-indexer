//! Asset dictionary. Tokenized stocks come from their issuers (Backed, Backpack, PreStocks), crypto majors and
//! yield tokens from a hand-checked seed list. Everything prices and swaps the same way, so it all lives in `stocks`.
use anyhow::Result;
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const BACKED: &str = "https://api.backed.fi/api/v2/public/assets";
const BACKPACK_ASSETS: &str = "https://api.backpack.exchange/api/v1/assets";
const JUPITER_SEARCH: &str = "https://lite-api.jup.ag/tokens/v2/search?query=";
const DEXSCREENER: &str = "https://api.dexscreener.com/tokens/v1/solana/";
// Exclusions and category overrides live in stock_config (migration 0009): flipping a row needs no deploy.

fn client() -> reqwest::Client {
    reqwest::Client::builder().user_agent("ape-indexer/0.1").timeout(Duration::from_secs(20)).build().unwrap()
}

/// One catalog row before it touches the database. `underlying` is the real-world ticker three issuers can share.
struct Asset { mint: String, symbol: String, name: String, issuer: &'static str, category: &'static str, underlying: String, logo: Option<String>, halted: bool, tags: Vec<String> }

const ETF_WORDS: [&str; 12] = ["ETF", "Trust", "Fund", "iShares", "SPDR", "Invesco", "ProShares", "Direxion", "Global X", "Roundhill", "Schwab", "Vanguard"];
const ETF_TICKERS: [&str; 17] = ["SPY", "QQQ", "TQQQ", "SQQQ", "GLD", "VTI", "IWM", "DIA", "USO", "URA", "COPX", "EWY", "DRAM", "SCHH", "SOXL", "IAU", "SLV"];
fn stock_or_etf(name: &str, underlying: &str) -> &'static str {
    if ETF_WORDS.iter().any(|w| name.contains(w)) || ETF_TICKERS.contains(&underlying) { "etf" } else { "stock" }
}

// ── Backed (xStocks): the issuer's public catalog, paginated, no key ──
#[derive(Deserialize)]
struct BackedDeployment { address: String, network: String }
#[derive(Deserialize)]
struct BackedAsset {
    symbol: String, name: String, logo: Option<String>,
    #[serde(rename = "underlyingSymbol")] underlying: Option<String>,
    #[serde(rename = "isTradingHalted")] halted: Option<bool>,
    deployments: Vec<BackedDeployment>,
}
#[derive(Deserialize)]
struct BackedPage { #[serde(rename = "hasNextPage")] has_next: bool }
#[derive(Deserialize)]
struct BackedList { nodes: Vec<BackedAsset>, page: BackedPage }

async fn backed(c: &reqwest::Client) -> Vec<Asset> {
    let mut out = Vec::new();
    for page in 1..=40 {
        let r: BackedList = match c.get(format!("{BACKED}?page={page}")).send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => match r.json().await { Ok(v) => v, Err(e) => { tracing::warn!(%e, page, "backed decode"); break } },
            Err(e) => { tracing::warn!(%e, page, "backed fetch"); break }
        };
        let more = r.page.has_next;
        for a in r.nodes {
            let Some(d) = a.deployments.iter().find(|d| d.network == "Solana") else { continue };
            let underlying = a.underlying.clone().unwrap_or_else(|| a.symbol.trim_end_matches('x').to_string());
            let name = a.name.trim_end_matches(" xStock").to_string();
            out.push(Asset { mint: d.address.clone(), category: stock_or_etf(&name, &underlying), symbol: a.symbol, name, issuer: "xstocks", underlying, logo: a.logo, halted: a.halted.unwrap_or(false), tags: vec![] });
        }
        if !more { break }
    }
    tracing::info!(n = out.len(), "backed assets");
    out
}

// ── Backpack Securities: every `.US` asset with a Solana mint in the exchange's public asset list ──
#[derive(Deserialize)]
struct BpToken { blockchain: String, #[serde(rename = "contractAddress")] address: Option<String> }
#[derive(Deserialize)]
struct BpAsset { symbol: String, #[serde(rename = "displayName")] name: String, tokens: Vec<BpToken> }

async fn backpack(c: &reqwest::Client) -> Vec<Asset> {
    let list: Vec<BpAsset> = match c.get(BACKPACK_ASSETS).send().await.and_then(|r| r.error_for_status()) {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => { tracing::warn!(%e, "backpack assets"); return vec![] }
    };
    let mut out = Vec::new();
    for a in list {
        let Some(under) = a.symbol.strip_suffix(".US") else { continue };
        let Some(mint) = a.tokens.iter().find(|t| t.blockchain == "Solana").and_then(|t| t.address.clone()) else { continue };
        let logo = Some(format!("https://backpack.exchange/api/stock-logo/{under}"));
        out.push(Asset { mint, symbol: under.to_string(), category: stock_or_etf(&a.name, under), name: a.name, issuer: "backpack", underlying: under.to_string(), logo, halted: false, tags: vec![] });
    }
    tracing::info!(n = out.len(), "backpack assets");
    out
}

// ── PreStocks: their own API is both the catalog and the mark feed ──
fn prestocks(marks: &HashMap<String, PreStock>) -> Vec<Asset> {
    marks.values().filter_map(|p| {
        let (symbol, name) = (p.symbol.clone()?, p.name.clone()?);
        let name = name.trim_end_matches(" PreStocks").to_string();
        Some(Asset { mint: p.contract_address.clone(), underlying: symbol.clone(), symbol, name, issuer: "prestocks", category: "preipo", logo: p.image.clone(), halted: false, tags: vec![] })
    }).collect()
}

// ── Crypto majors and yield tokens. Jupiter-verified and liquid, hand-checked: name search returns squatters
// for half of these, so the mint is the source of truth. (mint, symbol, name, category, group, underlying) ──
const SEEDS: [(&str, &str, &str, &str, &str, &str); 31] = [
    ("So11111111111111111111111111111111111111112", "SOL", "Solana", "crypto", "majors", "SOL"),
    ("3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh", "WBTC", "Wrapped BTC (Portal)", "crypto", "majors", "BTC"),
    ("cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij", "cbBTC", "Coinbase Wrapped BTC", "crypto", "majors", "BTC"),
    ("7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs", "ETH", "Ether (Portal)", "crypto", "majors", "ETH"),
    ("6UpQcMAb5xMzxc7ZfPaVMgx3KqsvKZdT5U718BzD5We2", "wXRP", "Wrapped XRP", "crypto", "majors", "XRP"),
    ("9gP2kCy3wA1ctvYWQk75guqXuHfrEomqydHLtcTCqiLa", "BNB", "Binance Coin (Portal)", "crypto", "majors", "BNB"),
    ("DoGEV7LASBkQbibMc5k5vKnTZoMg423GpJ5QtJEGfm7R", "DOGE", "Dogecoin", "crypto", "majors", "DOGE"),
    ("cbLTC4T5NpzSUtQ7ekgEMZGaUPVJY1ko6BUikqa4gGf", "cbLTC", "Coinbase Wrapped LTC", "crypto", "majors", "LTC"),
    ("avaxGHCq3T7hoxd73oY2KY9hJSTaeMibXvHy5KNzh5D", "AVAX", "Avalanche", "crypto", "l1", "AVAX"),
    ("suifhC9gU1VbJAPYPTBkHJyyyStKGLLYPVDTmPoqbvA", "SUI", "Sui", "crypto", "l1", "SUI"),
    ("3ZLekZYq2qkZiSpnSvabjit34tUkjSwD1JFuW9as9wBG", "wNEAR", "Wrapped NEAR", "crypto", "l1", "NEAR"),
    ("98sMhvDwXj1RQi5c5Mndm3vPe9cBqPrbLaufMXFNMh5g", "HYPE", "Hyperliquid", "crypto", "l1", "HYPE"),
    ("taoC6xyv2v8tDLcev4uaGUgV4vdQsWJrGft2kcBRrBY", "TAO", "Bittensor", "crypto", "l1", "TAO"),
    ("ARBzQTYDCW2KnVEjs1Mc81LekB1ibVFZKbSVmorkoT9d", "ARB", "Arbitrum", "crypto", "l1", "ARB"),
    ("LinkhB3afbBKb2EQQu7s7umdZceV3wcvAUJhQAfQ23L", "LINK", "Chainlink", "crypto", "defi", "LINK"),
    ("uniHfuPhEQSrtpzXpJZDCSq53yaejKKpNhFUiKoHKHV", "UNI", "Uniswap", "crypto", "defi", "UNI"),
    ("AavE1kKKnesPw4MuRJmJ9jZs9QzEE8CPxQ3ViczUDfc1", "AAVE", "Aave", "crypto", "defi", "AAVE"),
    ("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R", "RAY", "Raydium", "crypto", "defi", "RAY"),
    ("JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN", "JUP", "Jupiter", "crypto", "defi", "JUP"),
    ("jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL", "JTO", "Jito", "crypto", "defi", "JTO"),
    ("HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3", "PYTH", "Pyth Network", "crypto", "defi", "PYTH"),
    ("PEPEqnuuCDbBC89p1u9vpnP1KQ2oj1xTcQBsjt9X55m", "PEPE", "Pepe", "crypto", "memes", "PEPE"),
    ("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263", "BONK", "Bonk", "crypto", "memes", "BONK"),
    ("EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm", "WIF", "dogwifhat", "crypto", "memes", "WIF"),
    ("2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv", "PENGU", "Pudgy Penguins", "crypto", "memes", "PENGU"),
    ("6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN", "TRUMP", "Official Trump", "crypto", "memes", "TRUMP"),
    ("rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBof", "RENDER", "Render", "crypto", "solana", "RENDER"),
    ("hntyVP6YFm1Hg25TN9WGLqM12b8TQmcknKrdu1oxWux", "HNT", "Helium", "crypto", "solana", "HNT"),
    ("AvZZF1YaZDziPY2RCK4oJrRVrbN3mTD9NL24hPeaZeUj", "syrupUSDC", "Maple Syrup USDC", "earn", "earn", "syrupUSDC"),
    ("DEkqHyPN7GMRJ5cArtQFAWefqbZb33Hyf6s5iCwjEonT", "USDe", "Ethena USDe", "earn", "earn", "USDe"),
    ("A1KLoBrKBde8Ty9qtNQUtq3C2ortoC3u7twggz7sEto6", "USDY", "Ondo US Dollar Yield", "earn", "earn", "USDY"),
];

/// Logos for the seeds come from Jupiter's token search by mint; the upsert keeps an existing logo on a miss.
async fn seeds(c: &reqwest::Client) -> Vec<Asset> {
    let mut out = Vec::with_capacity(SEEDS.len());
    for (mint, symbol, name, category, group, underlying) in SEEDS {
        let logo = c.get(format!("{JUPITER_SEARCH}{mint}")).send().await.ok()
            .and_then(|r| r.error_for_status().ok());
        let logo = match logo { Some(r) => r.json::<Vec<serde_json::Value>>().await.ok().and_then(|v| v.into_iter().find(|t| t["id"] == mint).and_then(|t| t["icon"].as_str().map(String::from))), None => None };
        out.push(Asset { mint: mint.into(), symbol: symbol.into(), name: name.into(), issuer: if category == "earn" { "earn" } else { "crypto" }, category, underlying: underlying.into(), logo, halted: false, tags: vec![group.to_string()] });
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    out
}

/// Upsert the whole catalog. Returns the tokenized-stock mints the stream watches as quote tokens: crypto and
/// yield tokens are deliberately left out, SOL alone would subscribe the stream to the entire chain.
pub async fn sync_list(db: &PgPool) -> Result<HashSet<String>> {
    let c = client();
    let marks = prestocks_marks(&c).await;
    let mut assets = backed(&c).await;
    assets.extend(backpack(&c).await);
    assets.extend(prestocks(&marks));
    assets.extend(seeds(&c).await);
    if assets.len() < 500 { anyhow::bail!("catalog came back with {} assets; a source is down, keeping the table as is", assets.len()) }
    let now = chrono_now();
    let cfg: HashMap<String, (bool, Option<String>, Vec<String>)> = sqlx::query_as::<_, (String, bool, Option<String>, Vec<String>)>("SELECT mint, excluded, category, tags FROM stock_config")
        .fetch_all(db).await?.into_iter().map(|r| (r.0, (r.1, r.2, r.3))).collect();
    let mut watched = HashSet::new();
    let assets_mints: Vec<String> = assets.iter().map(|a| a.mint.clone()).collect();
    for a in assets {
        let (excluded, cat_override, mut tags) = cfg.get(&a.mint).cloned().unwrap_or((false, None, vec![]));
        tags.extend(a.tags);
        let cat = cat_override.unwrap_or_else(|| a.category.to_string());
        // decimals are filled from the mint account in refresh_mint_meta; 0 only ever exists between the two.
        sqlx::query("INSERT INTO stocks (mint, symbol, name, issuer, category, decimals, logo, updated_at, excluded, tags, halted, underlying) VALUES ($1,$2,$3,$4,$5,0,$6,$7,$8,$9,$10,$11)
                     ON CONFLICT (mint) DO UPDATE SET symbol=EXCLUDED.symbol, name=EXCLUDED.name, issuer=EXCLUDED.issuer, category=EXCLUDED.category, logo=COALESCE(EXCLUDED.logo, stocks.logo), excluded=EXCLUDED.excluded, tags=EXCLUDED.tags, halted=EXCLUDED.halted, underlying=EXCLUDED.underlying")
            .bind(&a.mint).bind(&a.symbol).bind(&a.name).bind(a.issuer).bind(&cat).bind(&a.logo).bind(now).bind(excluded).bind(&tags).bind(a.halted).bind(&a.underlying)
            .execute(db).await?;
        if !excluded && matches!(a.issuer, "xstocks" | "backpack" | "prestocks") { watched.insert(a.mint); }
    }
    // A row no issuer lists any more is hidden, not deleted: trades and orders still reference it.
    let synced: Vec<&str> = assets_mints.iter().map(String::as_str).collect();
    sqlx::query("UPDATE stocks SET excluded = true WHERE NOT excluded AND NOT (mint = ANY($1))").bind(&synced).execute(db).await?;
    tracing::info!(n = watched.len(), "stocks synced");
    if let Err(e) = refresh_mint_meta(db).await { tracing::warn!(%e, "mint meta") }
    Ok(watched)
}

/// Decimals and the Token-2022 scaled-UI multiplier, straight from the mint accounts, 100 per RPC call.
/// xStocks/Backpack pay dividends and do splits by raising the multiplier (1 raw unit = multiplier displayed
/// units), so every raw→USD conversion must include it. A pending `newMultiplier` takes over once its
/// effective time has passed. Mints without the extension stay at 1.
pub async fn refresh_mint_meta(db: &PgPool) -> Result<usize> {
    let rpc = crate::enrich::Rpc::new();
    let mints: Vec<String> = sqlx::query_scalar("SELECT mint FROM stocks").fetch_all(db).await?;
    let now = chrono_now(); let mut n = 0;
    for chunk in mints.chunks(100) {
        let r = match rpc.call("getMultipleAccounts", serde_json::json!([chunk, {"encoding":"jsonParsed"}])).await { Ok(r) => r, Err(e) => { tracing::warn!(%e, "mint meta rpc"); continue } };
        for (mint, acct) in chunk.iter().zip(r["value"].as_array().cloned().unwrap_or_default()) {
            let info = &acct["data"]["parsed"]["info"];
            let Some(decimals) = info["decimals"].as_i64() else { continue };
            let mut m = 1.0;
            for ext in info["extensions"].as_array().cloned().unwrap_or_default() {
                if ext["extension"].as_str() != Some("scaledUiAmountConfig") { continue }
                let st = &ext["state"];
                let cur = st["multiplier"].as_str().and_then(|x| x.parse::<f64>().ok()).or_else(|| st["multiplier"].as_f64()).unwrap_or(1.0);
                let new = st["newMultiplier"].as_str().and_then(|x| x.parse::<f64>().ok()).or_else(|| st["newMultiplier"].as_f64());
                let eff = st["newMultiplierEffectiveTimestamp"].as_i64().unwrap_or(i64::MAX);
                m = match new { Some(nm) if eff <= now && nm > 0.0 => nm, _ => cur };
            }
            if m <= 0.0 { m = 1.0 }
            sqlx::query("UPDATE stocks SET decimals = $2, multiplier = $3 WHERE mint = $1 AND (decimals IS DISTINCT FROM $2 OR multiplier IS DISTINCT FROM $3)")
                .bind(mint).bind(decimals as i16).bind(m).execute(db).await?;
            n += 1;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tracing::info!(n, "mint meta refreshed");
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
struct PreStock { #[serde(rename = "contract_address")] contract_address: String, symbol: Option<String>, name: Option<String>, image: Option<String>, mark_price: Option<f64>, mark_valuation: Option<f64>, implied_valuation: Option<f64>, supply: Option<f64> }

/// PreStocks' own numbers per mint: mark price of the underlying, valuations, supply. Empty map on any failure.
async fn prestocks_marks(c: &reqwest::Client) -> std::collections::HashMap<String, PreStock> {
    match c.get(PRESTOCKS).send().await.and_then(|r| r.error_for_status()) {
        Ok(r) => r.json::<Vec<PreStock>>().await.unwrap_or_default().into_iter().map(|p| (p.contract_address.clone(), p)).collect(),
        Err(e) => { tracing::warn!(%e, "prestocks api"); Default::default() }
    }
}

/// One pass of market data: 30 mints per DexScreener call, best pair by liquidity, plus PreStocks marks.
/// Updates the live columns on `stocks`; with `snapshot` also appends a row per stock to `stock_snapshots`.
/// `all` sweeps the whole catalog so a mint that just got its first pool is found; otherwise only pooled mints.
pub async fn refresh_prices(db: &PgPool, snapshot: bool, all: bool) -> Result<usize> {
    let mints: Vec<String> = if all { sqlx::query_scalar("SELECT mint FROM stocks").fetch_all(db).await? }
        else { sqlx::query_scalar("SELECT mint FROM stocks WHERE coalesce(liquidity_usd, 0) > 0").fetch_all(db).await? };
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

/// Every 30s: live columns for pooled mints. Every other pass (60s): a snapshot row. Once an hour: the whole
/// catalog, for pool discovery, and drop snapshots older than 30 days.
pub async fn price_loop(db: PgPool) {
    let mut pass: u64 = 0;
    loop {
        let last = std::time::Instant::now();
        match refresh_prices(&db, pass % 2 == 0, pass % 120 == 0).await { Ok(n) => { crate::metrics::stock_prices_refreshed(); tracing::info!(n, "stock prices refreshed") }, Err(e) => tracing::warn!(%e, "price loop") }
        if pass % 120 == 0 {
            let cutoff = chrono_now() - 30 * 86400;
            if let Err(e) = sqlx::query("DELETE FROM stock_snapshots WHERE ts < $1").bind(cutoff).execute(&db).await { tracing::warn!(%e, "snapshot retention") }
        }
        pass += 1;
        for _ in 0..30 { tokio::time::sleep(Duration::from_secs(1)).await; crate::metrics::stock_prices_age(last.elapsed().as_secs_f64()); }
    }
}

/// Every 5s: Jupiter price for every stock → stock_ticks (24h retention) and a `price` frame to the stock's room.
/// Pushes every pass (heartbeat). Feeds /history 5m/15m/1h and the live hero price.
pub async fn tick_loop(db: PgPool, push: Option<crate::push::Pusher>) {
    let c = client(); let mut pass: u64 = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let rows: Vec<(String, Option<f64>, Option<f64>)> = match sqlx::query_as("SELECT mint, mark_usd, change_24h FROM stocks WHERE NOT excluded AND coalesce(liquidity_usd, 0) >= 1000").fetch_all(&db).await { Ok(r) => r, Err(e) => { tracing::warn!(%e, "ticks: stocks"); continue } };
        let mints: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let jup = jupiter_prices(&c, &mints).await;
        if jup.is_empty() { continue }
        let now = chrono_now();
        for (mint, mark, chg) in &rows {
            let Some(j) = jup.get(mint) else { continue };
            if let Err(e) = sqlx::query("INSERT INTO stock_ticks (mint, ts, price_usd) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING").bind(mint).bind(now).bind(j.usd).execute(&db).await { tracing::warn!(%e, "ticks: insert"); }
            // Always push, even when unchanged: the frame doubles as a heartbeat so a fresh subscriber sees a price within 5s.
            if let Some(p) = &push { p.send_price(crate::push::IngestPrice { mint: mint.clone(), kind: "stock", ts: now, price_usd: j.usd, mark_usd: *mark, change_24h: *chg }); }
        }
        pass += 1;
        if pass % 720 == 0 { if let Err(e) = sqlx::query("DELETE FROM stock_ticks WHERE ts < $1").bind(now - 86400).execute(&db).await { tracing::warn!(%e, "ticks: retention") } }
    }
}

pub fn chrono_now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 }
