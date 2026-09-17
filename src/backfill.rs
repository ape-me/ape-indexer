//! One-time catalog backfill from public APIs: every existing stonk token and its candle history.
use anyhow::Result;
use bigdecimal::BigDecimal;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;

const RAY_LIST: &str = "https://launch-mint-v1.raydium.io/get/list";
const RAY_KLINE: &str = "https://launch-history-v1.raydium.io/kline";
const PUMP_COINS: &str = "https://frontend-api-v3.pump.fun/coins";
const PUMP_CANDLES: &str = "https://swap-api.pump.fun/v1/coins";
const DEXSCREENER_PAIRS: &str = "https://api.dexscreener.com/token-pairs/v1/solana";
pub const STONKFUN_PLATFORMS: [&str; 2] = ["6BwHHDg3u1854jC8PDLXvR4spTcLNaoBxLJNGC4nTESt", "4E876qZTE9FJMrBzgVtBrSrzz2TLivB5Y5QXPjB4gZL7"];

fn http() -> reqwest::Client { reqwest::Client::builder().user_agent("Mozilla/5.0 ape-indexer").timeout(Duration::from_secs(20)).build().unwrap() }
async fn get(c: &reqwest::Client, url: &str) -> Result<Value> {
    let mut last = None;
    for attempt in 0..4 {
        match c.get(url).send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => match r.json::<Value>().await { Ok(v) => return Ok(v), Err(e) => last = Some(anyhow::anyhow!(e)) },
            Err(e) => last = Some(anyhow::anyhow!(e)),
        }
        tokio::time::sleep(Duration::from_millis(500 * (1 << attempt))).await;
    }
    Err(last.unwrap())
}
fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> { v[k].as_str().filter(|x| !x.is_empty()) }

async fn stock_set(db: &PgPool) -> Result<HashSet<String>> { Ok(sqlx::query_scalar::<_, String>("SELECT mint FROM stocks").fetch_all(db).await?.into_iter().collect()) }

async fn upsert_token(db: &PgPool, mint: &str, symbol: Option<&str>, name: Option<&str>, image: Option<&str>, uri: Option<&str>, quote: &str, launchpad: &str, creator: Option<&str>,
                      decimals: i16, supply: Option<&str>, graduated: bool, curve_pool: &str, amm_pool: Option<&str>, tax_bps: i32, created_at: i64) -> Result<()> {
    let supply = supply.and_then(|x| BigDecimal::from_str(x).ok());
    sqlx::query("INSERT INTO tokens (mint, symbol, name, image, uri, quote_mint, launchpad, creator, decimals, supply, phase, curve_pool, amm_pool, tax_bps, created_at, source)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,'backfill')
                 ON CONFLICT (mint) DO UPDATE SET symbol=COALESCE(tokens.symbol,EXCLUDED.symbol), name=COALESCE(tokens.name,EXCLUDED.name), image=COALESCE(NULLIF(tokens.image,''),EXCLUDED.image),
                   uri=COALESCE(tokens.uri,EXCLUDED.uri), supply=COALESCE(tokens.supply,EXCLUDED.supply), creator=COALESCE(tokens.creator,EXCLUDED.creator),
                   phase=CASE WHEN EXCLUDED.phase='graduated' THEN 'graduated' ELSE tokens.phase END, amm_pool=COALESCE(tokens.amm_pool,EXCLUDED.amm_pool),
                   tax_bps=GREATEST(tokens.tax_bps,EXCLUDED.tax_bps), created_at=LEAST(tokens.created_at,EXCLUDED.created_at)")
        .bind(mint).bind(symbol).bind(name).bind(image).bind(uri).bind(quote).bind(launchpad).bind(creator).bind(decimals).bind(supply)
        .bind(if graduated { "graduated" } else { "curve" }).bind(curve_pool).bind(amm_pool).bind(tax_bps).bind(created_at).execute(db).await?;
    sqlx::query("INSERT INTO pools (pool, token_mint, program, kind, created_at) VALUES ($1,$2,$3,'curve',$4) ON CONFLICT DO NOTHING")
        .bind(curve_pool).bind(mint).bind(match launchpad { "stonkfun" => "launchlab", "pumpfun" => "pumpfun", _ => "dbc" }).bind(created_at).execute(db).await?;
    if let Some(p) = amm_pool {
        sqlx::query("INSERT INTO pools (pool, token_mint, program, kind, created_at) VALUES ($1,$2,$3,'amm',$4) ON CONFLICT DO NOTHING")
            .bind(p).bind(mint).bind(match launchpad { "stonkfun" => "cpmm", "pumpfun" => "pumpswap", _ => "damm2" }).bind(created_at).execute(db).await?;
    }
    sqlx::query("INSERT INTO token_stats (token_mint, updated_at) VALUES ($1,$2) ON CONFLICT DO NOTHING").bind(mint).bind(created_at).execute(db).await?;
    Ok(())
}

/// StonkFun tokens from Raydium's LaunchLab list, both platform ids, newest first, all pages.
pub async fn raydium(db: &PgPool, max_pages: usize) -> Result<usize> {
    let c = http(); let stocks = stock_set(db).await?; let mut n = 0;
    for platform in STONKFUN_PLATFORMS {
        let mut next: Option<String> = None;
        for _ in 0..max_pages {
            let url = format!("{RAY_LIST}?platformId={platform}&sort=new&size=100&mintType=default&includeNsfw=true{}", next.as_ref().map(|p| format!("&nextPageId={p}")).unwrap_or_default());
            let v = match get(&c, &url).await { Ok(v) => v, Err(e) => { tracing::warn!(%e, platform, "raydium page failed, moving on"); break } }; let d = &v["data"];
            let rows = d["rows"].as_array().cloned().unwrap_or_default();
            if rows.is_empty() { break; }
            for r in &rows {
                let Some(quote) = s(&r["mintB"], "address") else { continue };
                if !stocks.contains(quote) { continue; }
                let (Some(mint), Some(pool)) = (s(r, "mint"), s(r, "poolId")) else { continue };
                let graduated = r["finishingRate"].as_f64().unwrap_or(0.0) >= 100.0 || s(r, "migrateAmmId").is_some();
                let supply = r["supply"].as_f64().map(|x| format!("{:.0}", x * 10f64.powi(r["decimals"].as_i64().unwrap_or(6) as i32)));
                upsert_token(db, mint, s(r, "symbol"), s(r, "name"), s(r, "imgUrl"), s(r, "metadataUrl"), quote, "stonkfun", s(r, "creator"),
                    r["decimals"].as_i64().unwrap_or(6) as i16, supply.as_deref(), graduated, pool, s(r, "migrateAmmId"),
                    r["transferFeeBasePoints"].as_i64().unwrap_or(0) as i32, r["createAt"].as_i64().unwrap_or(0) / 1000).await?;
                n += 1;
            }
            next = s(d, "nextPageId").map(String::from);
            if next.is_none() { break; }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    tracing::info!(n, "raydium backfill");
    Ok(n)
}

/// pump.fun coins with a stock quote mint. Newest first; stops after `max_pages` of 50.
pub async fn pump(db: &PgPool, max_pages: usize) -> Result<usize> {
    let c = http(); let stocks = stock_set(db).await?; let mut n = 0;
    for page in 0..max_pages {
        let url = format!("{PUMP_COINS}?offset={}&limit=50&sort=created_timestamp&order=DESC&includeNsfw=true", page * 50);
        let rows = get(&c, &url).await?.as_array().cloned().unwrap_or_default();
        if rows.is_empty() { break; }
        for r in &rows {
            let Some(quote) = s(r, "quote_mint") else { continue };
            if !stocks.contains(quote) { continue; }
            let (Some(mint), Some(curve)) = (s(r, "mint"), s(r, "bonding_curve")) else { continue };
            let complete = r["complete"].as_bool().unwrap_or(false);
            upsert_token(db, mint, s(r, "symbol"), s(r, "name"), s(r, "image_uri"), s(r, "metadata_uri"), quote, "pumpfun", s(r, "creator"),
                r["base_decimals"].as_i64().unwrap_or(6) as i16, s(r, "total_supply_str"), complete, curve, s(r, "pump_swap_pool"),
                0, r["created_timestamp"].as_i64().unwrap_or(0) / 1000).await?;
            n += 1;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    tracing::info!(n, "pump backfill");
    Ok(n)
}

/// Meteora DBC stonk pools via DexScreener: pairs per stock mint where dexId is meteoradbc and the stock is the quote.
pub async fn dbc(db: &PgPool) -> Result<usize> {
    let c = http(); let stocks = stock_set(db).await?; let mut n = 0;
    for stock in &stocks {
        let pairs = match get(&c, &format!("{DEXSCREENER_PAIRS}/{stock}")).await { Ok(v) => v.as_array().cloned().unwrap_or_default(), Err(_) => continue };
        for p in &pairs {
            if p["dexId"].as_str() != Some("meteoradbc") || s(&p["quoteToken"], "address") != Some(stock.as_str()) { continue; }
            let (Some(mint), Some(pool)) = (s(&p["baseToken"], "address"), s(p, "pairAddress")) else { continue };
            upsert_token(db, mint, s(&p["baseToken"], "symbol"), s(&p["baseToken"], "name"), s(&p["info"], "imageUrl"), None, stock, "dbc", None, 6, None, false, pool, None, 0,
                p["pairCreatedAt"].as_i64().unwrap_or(0) / 1000).await?;
            n += 1;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    tracing::info!(n, "dbc backfill");
    Ok(n)
}

async fn put_candle(db: &PgPool, mint: &str, t: i64, o: f64, h: f64, l: f64, c: f64, vol: f64) -> Result<()> {
    sqlx::query("INSERT INTO candles_1m (token_mint, minute, o, h, l, c, vol_quote, n) VALUES ($1,$2,$3,$4,$5,$6,$7,0) ON CONFLICT (token_mint, minute) DO NOTHING")
        .bind(mint).bind(t - t.rem_euclid(60)).bind(o).bind(h).bind(l).bind(c).bind(vol).execute(db).await?;
    Ok(())
}

/// 1m candle history for curve pools. Raydium klines carry no volume. Up to `max_per_pool` candles each.
pub async fn candles(db: &PgPool, limit_tokens: i64, max_per_pool: usize) -> Result<usize> {
    let c = http(); let mut n = 0;
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT t.mint, t.launchpad, t.curve_pool FROM tokens t WHERE t.curve_pool IS NOT NULL AND NOT EXISTS (SELECT 1 FROM candles_1m c WHERE c.token_mint = t.mint AND c.n = 0)
         ORDER BY t.created_at DESC LIMIT $1").bind(limit_tokens).fetch_all(db).await?;
    for (mint, launchpad, pool) in rows {
        let mut got = 0usize;
        match launchpad.as_str() {
            "stonkfun" => {
                let mut next: Option<String> = None;
                while got < max_per_pool {
                    let url = format!("{RAY_KLINE}?poolId={pool}&interval=1m&limit=100{}", next.as_ref().map(|k| format!("&nextPageKey={k}")).unwrap_or_default());
                    let Ok(v) = get(&c, &url).await else { break };
                    let rows = v["data"]["rows"].as_array().cloned().unwrap_or_default();
                    if rows.is_empty() { break; }
                    for k in &rows { put_candle(db, &mint, k["t"].as_i64().unwrap_or(0), k["o"].as_f64().unwrap_or(0.0), k["h"].as_f64().unwrap_or(0.0), k["l"].as_f64().unwrap_or(0.0), k["c"].as_f64().unwrap_or(0.0), 0.0).await?; }
                    got += rows.len();
                    next = s(&v["data"], "nextPageKey").map(String::from);
                    if next.is_none() { break; }
                }
            }
            "pumpfun" => {
                // pump.fun candles are in USD. Convert to the quote stock's units with its current USD price.
                let usd: Option<f64> = sqlx::query_scalar("SELECT s.price_usd FROM tokens t JOIN stocks s ON s.mint=t.quote_mint WHERE t.mint=$1").bind(&mint).fetch_optional(db).await?.flatten();
                let Some(usd) = usd.filter(|u| *u > 0.0) else { continue };
                let url = format!("{PUMP_CANDLES}/{mint}/candles?interval=1m&limit={}", max_per_pool.min(1000));
                if let Ok(v) = get(&c, &url).await {
                    for k in v.as_array().cloned().unwrap_or_default() {
                        let f = |key: &str| k[key].as_str().and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0) / usd;
                        put_candle(db, &mint, k["timestamp"].as_i64().unwrap_or(0) / 1000, f("open"), f("high"), f("low"), f("close"), f("volume")).await?;
                        got += 1;
                    }
                }
            }
            _ => {}
        }
        if got == 0 { put_candle(db, &mint, 0, 0.0, 0.0, 0.0, 0.0, 0.0).await?; } // sentinel so we don't retry forever
        // seed price from the newest candle when there is no live trade yet
        sqlx::query("UPDATE token_stats ts SET price_quote = c.c FROM (SELECT c FROM candles_1m WHERE token_mint=$1 AND minute>0 ORDER BY minute DESC LIMIT 1) c WHERE ts.token_mint=$1 AND ts.price_quote IS NULL").bind(&mint).execute(db).await?;
        n += 1;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    tracing::info!(n, "candle backfill");
    Ok(n)
}
