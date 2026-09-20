//! Applies events to Postgres, folds candles, keeps token_stats current, pushes trades to ape-be.
use crate::events::{Event, Side};
use anyhow::Result;
use bigdecimal::BigDecimal;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

#[derive(Clone, Debug)]
pub struct TokenInfo { pub decimals: i16, pub quote_mint: String, pub supply: Option<f64> }

pub struct Store {
    pub db: PgPool,
    rpc: crate::enrich::Rpc,
    push: Option<crate::push::Pusher>,
    pub stocks: HashMap<String, i16>,          // mint -> decimals
    stock_usd: HashMap<String, f64>,
    tokens: HashMap<String, TokenInfo>,        // token mint -> info
    pools: HashMap<String, String>,            // pool -> token mint
}

fn symbol_c(t: &Option<crate::events::TokenMeta>) -> Option<String> { t.as_ref().map(|t| t.symbol.trim_end_matches('\0').trim().to_string()) }
fn name_c(t: &Option<crate::events::TokenMeta>) -> Option<String> { t.as_ref().map(|t| t.name.trim_end_matches('\0').trim().to_string()) }

impl Store {
    /// `push` = (ingest url, hmac secret) of ape-be; None disables live fan-out.
    pub async fn open(db: PgPool, push: Option<(String, String)>) -> Result<Store> {
        let push = push.map(|(u, k)| crate::push::Pusher::start(u, k));
        let mut s = Store { db, push, rpc: crate::enrich::Rpc::new(), stocks: HashMap::new(), stock_usd: HashMap::new(), tokens: HashMap::new(), pools: HashMap::new() };
        s.reload().await?;
        Ok(s)
    }

    /// Load dictionaries from Postgres. Cheap; called at start and every minute.
    pub async fn reload(&mut self) -> Result<()> {
        let rows: Vec<(String, i16, Option<f64>)> = sqlx::query_as("SELECT mint, decimals, price_usd FROM stocks").fetch_all(&self.db).await?;
        self.stocks = rows.iter().map(|r| (r.0.clone(), r.1)).collect();
        self.stock_usd = rows.iter().filter_map(|r| r.2.map(|p| (r.0.clone(), p))).collect();
        let toks: Vec<(String, i16, String, Option<BigDecimal>)> = sqlx::query_as("SELECT mint, decimals, quote_mint, supply FROM tokens").fetch_all(&self.db).await?;
        self.tokens = toks.into_iter().map(|t| (t.0, TokenInfo { decimals: t.1, quote_mint: t.2, supply: t.3.and_then(|b| f64::from_str(&b.to_string()).ok()) })).collect();
        let pools: Vec<(String, String)> = sqlx::query_as("SELECT pool, token_mint FROM pools").fetch_all(&self.db).await?;
        self.pools = pools.into_iter().collect();
        Ok(())
    }

    pub fn stock_set(&self) -> HashSet<String> { self.stocks.keys().cloned().collect() }

    pub async fn apply(&mut self, ev: &Event) -> Result<bool> {
        match ev {
            Event::PoolCreated { meta, pool, base_mint, quote_mint, creator, base_vault, quote_vault, token, holder_rewards: _ } => {
                let kind = if meta.program.is_curve() { "curve" } else { "amm" };
                if meta.program.is_curve() {
                    let decimals = token.as_ref().and_then(|t| t.decimals).unwrap_or(6) as i16;
                    let (name, symbol, uri) = token.as_ref().map(|t| (Some(t.name.trim_end_matches('\0').to_string()), Some(t.symbol.trim_end_matches('\0').to_string()), Some(t.uri.trim_end_matches('\0').to_string()))).unwrap_or((None, None, None));
                    sqlx::query("INSERT INTO tokens (mint, symbol, name, uri, quote_mint, launchpad, creator, decimals, phase, curve_pool, created_at, source)
                                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'curve',$9,$10,'stream') ON CONFLICT (mint) DO NOTHING")
                        .bind(base_mint).bind(symbol).bind(name).bind(uri).bind(quote_mint).bind(meta.program.launchpad()).bind(creator).bind(decimals).bind(pool).bind(meta.block_time)
                        .execute(&self.db).await?;
                    sqlx::query("INSERT INTO token_stats (token_mint, updated_at) VALUES ($1,$2) ON CONFLICT DO NOTHING").bind(base_mint).bind(meta.block_time).execute(&self.db).await?;
                    self.tokens.entry(base_mint.clone()).or_insert(TokenInfo { decimals, quote_mint: quote_mint.clone(), supply: None });
                    if let Some(p) = &self.push {
                        p.send_token(crate::push::IngestToken { event: "created", mint: base_mint.clone(), symbol: symbol_c(token), name: name_c(token), quote_mint: quote_mint.clone(), launchpad: meta.program.launchpad(), creator: Some(creator.clone()), created_at: meta.block_time, ts: meta.block_time });
                    }
                } else if self.tokens.contains_key(base_mint) {
                    // graduation: an AMM pool for a token we know
                    sqlx::query("UPDATE tokens SET phase='graduated', amm_pool=$2 WHERE mint=$1 AND amm_pool IS NULL").bind(base_mint).bind(pool).execute(&self.db).await?;
                    sqlx::query("UPDATE pools SET migrated_to=$2 WHERE token_mint=$1 AND kind='curve' AND migrated_to IS NULL").bind(base_mint).bind(pool).execute(&self.db).await?;
                    if let Some(p) = &self.push {
                        let qm = self.tokens.get(base_mint).map(|t| t.quote_mint.clone()).unwrap_or_else(|| quote_mint.clone());
                        p.send_token(crate::push::IngestToken { event: "graduated", mint: base_mint.clone(), symbol: None, name: None, quote_mint: qm, launchpad: meta.program.launchpad(), creator: None, created_at: 0, ts: meta.block_time });
                    }
                } else {
                    return Ok(false); // stock/SOL, stock/USDC and other pools we don't care about
                }
                sqlx::query("INSERT INTO pools (pool, token_mint, program, kind, base_vault, quote_vault, created_at) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (pool) DO NOTHING")
                    .bind(pool).bind(base_mint).bind(meta.program.name()).bind(kind).bind(base_vault).bind(quote_vault).bind(meta.block_time).execute(&self.db).await?;
                self.pools.insert(pool.clone(), base_mint.clone());
                Ok(true)
            }
            Event::Swap { meta, pool, base_mint, quote_mint, wallet, side, base_raw, quote_raw, progress_pct, .. } => {
                let info = match self.tokens.get(base_mint) {
                    Some(i) => i.clone(),
                    None if meta.program.is_curve() => {
                        // trade on a curve we never saw created (started mid-life). Stub the token; enrich later.
                        let mi = self.rpc.mint_info(base_mint).await.ok().flatten();
                        let decimals = mi.as_ref().map(|m| m.decimals).unwrap_or(6);
                        let supply = mi.as_ref().and_then(|m| f64::from_str(&m.supply).ok());
                        let info = TokenInfo { decimals, quote_mint: quote_mint.clone(), supply };
                        sqlx::query("INSERT INTO tokens (mint, quote_mint, launchpad, decimals, supply, phase, curve_pool, created_at, source) VALUES ($1,$2,$3,$4,$5,'curve',$6,$7,'stub') ON CONFLICT (mint) DO NOTHING")
                            .bind(base_mint).bind(quote_mint).bind(meta.program.launchpad()).bind(decimals).bind(mi.as_ref().and_then(|m| BigDecimal::from_str(&m.supply).ok())).bind(pool).bind(meta.block_time).execute(&self.db).await?;
                        sqlx::query("INSERT INTO token_stats (token_mint, updated_at) VALUES ($1,$2) ON CONFLICT DO NOTHING").bind(base_mint).bind(meta.block_time).execute(&self.db).await?;
                        self.tokens.insert(base_mint.clone(), info.clone());
                        info
                    }
                    None => return Ok(false),
                };
                if !self.pools.contains_key(pool) {
                    let kind = if meta.program.is_curve() { "curve" } else { "amm" };
                    sqlx::query("INSERT INTO pools (pool, token_mint, program, kind, created_at) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (pool) DO NOTHING")
                        .bind(pool).bind(base_mint).bind(meta.program.name()).bind(kind).bind(meta.block_time).execute(&self.db).await?;
                    if kind == "amm" { sqlx::query("UPDATE tokens SET phase='graduated', amm_pool=$2 WHERE mint=$1 AND amm_pool IS NULL").bind(base_mint).bind(pool).execute(&self.db).await?; }
                    self.pools.insert(pool.clone(), base_mint.clone());
                }
                let qd = *self.stocks.get(quote_mint).unwrap_or(&6) as i32;
                let bd = info.decimals as i32;
                let scale = 10f64.powi(bd - qd);
                let Some(raw_price) = ev.raw_price() else { return Ok(false) };
                let price_quote = raw_price * scale;
                let base = *base_raw as f64 / 10f64.powi(bd);
                let quote = *quote_raw as f64 / 10f64.powi(qd);
                let minute = meta.block_time - meta.block_time.rem_euclid(60);
                let usd = self.stock_usd.get(quote_mint).copied();
                let price_usd = usd.map(|u| price_quote * u);
                let mcap = price_usd.and_then(|p| info.supply.map(|s| p * s / 10f64.powi(bd)));
                // One round trip: insert the trade, and only if it was new (not a replay), fold the candle and refresh stats.
                let ins: (i64,) = sqlx::query_as(
                    "WITH t AS (
                       INSERT INTO trades (signature, ix_index, slot, block_time, pool, token_mint, wallet, side, base_raw, quote_raw, price_quote, quote_usd)
                       VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$17) ON CONFLICT DO NOTHING RETURNING 1
                     ), c AS (
                       INSERT INTO candles_1m (token_mint, minute, o, h, l, c, vol_quote, n)
                       SELECT $6, $12, $11, $11, $11, $11, $13, 1 WHERE EXISTS (SELECT 1 FROM t)
                       ON CONFLICT (token_mint, minute) DO UPDATE SET h=GREATEST(candles_1m.h,$11), l=LEAST(candles_1m.l,$11), c=$11, vol_quote=candles_1m.vol_quote+$13, n=candles_1m.n+1
                     ), s AS (
                       INSERT INTO token_stats (token_mint, price_quote, price_usd, mcap_usd, ath_mcap_usd, progress_pct, last_trade_at, updated_at)
                       SELECT $6, $11, $14, $15, $15, $16, $4, $4 WHERE EXISTS (SELECT 1 FROM t)
                       ON CONFLICT (token_mint) DO UPDATE SET price_quote=$11, price_usd=$14, mcap_usd=COALESCE($15, token_stats.mcap_usd),
                         ath_mcap_usd=GREATEST(COALESCE(token_stats.ath_mcap_usd, 0), COALESCE($15, 0)), progress_pct=COALESCE($16, token_stats.progress_pct), last_trade_at=$4, updated_at=$4
                     )
                     SELECT count(*) FROM t")
                    .bind(&meta.signature).bind(meta.ix_index as i16).bind(meta.slot as i64).bind(meta.block_time).bind(pool).bind(base_mint).bind(wallet)
                    .bind(match side { Side::Buy => "buy", Side::Sell => "sell" }).bind(BigDecimal::from(*base_raw)).bind(BigDecimal::from(*quote_raw)).bind(price_quote)
                    .bind(minute).bind(quote).bind(price_usd).bind(mcap).bind(*progress_pct).bind(usd)
                    .fetch_one(&self.db).await?;
                if ins.0 == 0 { return Ok(false); } // replayed duplicate
                if let Some(p) = &self.push {
                    p.send(crate::push::IngestTrade { mint: base_mint.clone(), pool: pool.clone(), program: meta.program, sig: meta.signature.clone(), ts: meta.block_time, slot: meta.slot, side: *side, wallet: wallet.clone(), base, quote, price_quote, price_usd, quote_mint: quote_mint.clone() });
                }
                Ok(true)
            }
        }
    }

    /// 24h rollups for every token that traded recently. Runs once a minute.
    pub async fn rollup(db: &PgPool) -> Result<u64> {
        let now = crate::stocks::chrono_now();
        let r = sqlx::query(
            "WITH w AS (
               -- USD at trade time (quote_usd), falling back to the stock's current price for rows without it
               SELECT tr.token_mint,
                      SUM(tr.quote_raw::float8 / POWER(10, s.decimals) * COALESCE(tr.quote_usd, s.price_usd)) AS vol_usd,
                      COUNT(*) FILTER (WHERE tr.side='buy') AS buys,
                      COUNT(*) FILTER (WHERE tr.side='sell') AS sells
               FROM trades tr JOIN tokens tk ON tk.mint = tr.token_mint JOIN stocks s ON s.mint = tk.quote_mint
               WHERE tr.block_time > $1 - 86400 GROUP BY tr.token_mint),
             w1 AS (
               SELECT tr.token_mint,
                      SUM(tr.quote_raw::float8 / POWER(10, s.decimals) * COALESCE(tr.quote_usd, s.price_usd)) AS vol_usd,
                      COUNT(*) FILTER (WHERE tr.side='buy') AS buys, COUNT(*) FILTER (WHERE tr.side='sell') AS sells
               FROM trades tr JOIN tokens tk ON tk.mint = tr.token_mint JOIN stocks s ON s.mint = tk.quote_mint
               WHERE tr.block_time > $1 - 3600 GROUP BY tr.token_mint),
             p24 AS (
               -- price 24h ago; for tokens younger than 24h (or whose history starts later) the earliest candle's open
               SELECT DISTINCT ON (token_mint) token_mint,
                      CASE WHEN minute <= $1 - 86400 THEN c ELSE o END AS price_then
               FROM candles_1m
               ORDER BY token_mint, (minute <= $1 - 86400) DESC, CASE WHEN minute <= $1 - 86400 THEN -minute ELSE minute END),
             p1 AS (
               SELECT DISTINCT ON (token_mint) token_mint,
                      CASE WHEN minute <= $1 - 3600 THEN c ELSE o END AS price_then
               FROM candles_1m
               ORDER BY token_mint, (minute <= $1 - 3600) DESC, CASE WHEN minute <= $1 - 3600 THEN -minute ELSE minute END)
             UPDATE token_stats ts SET
               vol_24h_usd = COALESCE(w.vol_usd, 0),
               buys_24h = COALESCE(w.buys, 0), sells_24h = COALESCE(w.sells, 0),
               change_24h = CASE WHEN p24.price_then > 0 THEN (ts.price_quote / p24.price_then - 1) * 100 END,
               vol_1h_usd = COALESCE(w1.vol_usd, 0),
               buys_1h = COALESCE(w1.buys, 0), sells_1h = COALESCE(w1.sells, 0),
               change_1h = CASE WHEN p1.price_then > 0 THEN (ts.price_quote / p1.price_then - 1) * 100 END,
               price_usd = ts.price_quote * s.price_usd,
               mcap_usd = CASE WHEN t.supply IS NOT NULL THEN ts.price_quote * s.price_usd * t.supply::float8 / POWER(10, t.decimals) ELSE ts.mcap_usd END,
               updated_at = $1
             FROM tokens t JOIN stocks s ON s.mint = t.quote_mint
             LEFT JOIN w ON w.token_mint = t.mint
             LEFT JOIN w1 ON w1.token_mint = t.mint
             LEFT JOIN p24 ON p24.token_mint = t.mint
             LEFT JOIN p1 ON p1.token_mint = t.mint
             WHERE ts.token_mint = t.mint AND (ts.last_trade_at > $1 - 90000 OR w.token_mint IS NOT NULL)")
            .bind(now).execute(db).await?;
        Self::rollup_stocks(db, now).await?;
        Ok(r.rows_affected())
    }

    /// Per-floor 24h numbers (launches, volume, wallets, heat, king). One statement, every 60s, read by /api/stocks.
    async fn rollup_stocks(db: &PgPool, now: i64) -> Result<()> {
        sqlx::query(
            "WITH f AS (
               SELECT s.mint, COUNT(k.mint) FILTER (WHERE k.created_at > $1 - 86400)::int AS launched
               FROM stocks s LEFT JOIN tokens k ON k.quote_mint = s.mint GROUP BY s.mint),
             v AS (
               SELECT k.quote_mint AS mint,
                      SUM(tr.quote_raw::float8 / POWER(10, s.decimals) * COALESCE(tr.quote_usd, s.price_usd)) AS vol,
                      COUNT(*)::int AS trades, COUNT(DISTINCT tr.wallet)::int AS wallets
               FROM trades tr JOIN tokens k ON k.mint = tr.token_mint JOIN stocks s ON s.mint = k.quote_mint
               WHERE tr.block_time > $1 - 86400 GROUP BY k.quote_mint),
             king AS (
               SELECT DISTINCT ON (k.quote_mint) k.quote_mint AS mint, k.mint AS king_mint
               FROM tokens k JOIN token_stats st ON st.token_mint = k.mint
               WHERE st.vol_24h_usd > 0 ORDER BY k.quote_mint, st.vol_24h_usd DESC)
             INSERT INTO stock_stats (mint, launched_24h, meme_vol_24h, trades_24h, wallets_24h, heat, king_mint, updated_at)
             SELECT f.mint, f.launched, COALESCE(v.vol, 0), COALESCE(v.trades, 0), COALESCE(v.wallets, 0),
                    f.launched * 10 + COALESCE(v.wallets, 0) + COALESCE(v.vol, 0) / 1000, king.king_mint, $1
             FROM f LEFT JOIN v ON v.mint = f.mint LEFT JOIN king ON king.mint = f.mint
             ON CONFLICT (mint) DO UPDATE SET launched_24h = EXCLUDED.launched_24h, meme_vol_24h = EXCLUDED.meme_vol_24h,
               trades_24h = EXCLUDED.trades_24h, wallets_24h = EXCLUDED.wallets_24h, heat = EXCLUDED.heat,
               king_mint = EXCLUDED.king_mint, updated_at = EXCLUDED.updated_at")
            .bind(now).execute(db).await?;
        Ok(())
    }

    /// 5-minute window for tokens that traded in the last 6 minutes. Runs every 15s.
    pub async fn rollup_fast(db: &PgPool) -> Result<u64> {
        let now = crate::stocks::chrono_now();
        let r = sqlx::query(
            "WITH w AS (
               SELECT token_mint, SUM(quote_raw::float8) AS vol_q,
                      COUNT(*) FILTER (WHERE side='buy') AS buys, COUNT(*) FILTER (WHERE side='sell') AS sells
               FROM trades WHERE block_time > $1 - 300 GROUP BY token_mint)
             UPDATE token_stats ts SET
               vol_5m_usd = COALESCE(w.vol_q / POWER(10, s.decimals) * s.price_usd, 0),
               buys_5m = COALESCE(w.buys, 0), sells_5m = COALESCE(w.sells, 0)
             FROM tokens t JOIN stocks s ON s.mint = t.quote_mint
             LEFT JOIN w ON w.token_mint = t.mint
             WHERE ts.token_mint = t.mint AND (w.token_mint IS NOT NULL OR (ts.last_trade_at > $1 - 400 AND ts.vol_5m_usd > 0))")
            .bind(now).execute(db).await?;
        Ok(r.rows_affected())
    }

    /// Holder analysis from our own tape: net position per wallet = buys - sells. Exact for tokens indexed since birth
    /// (source='stream'), ignores plain transfers. Pool/curve accounts never appear as wallets, so nothing to exclude.
    /// Snipers = wallets whose first buy landed within 10s of the token's creation.
    pub async fn rollup_holders(db: &PgPool, max_tokens: i64) -> Result<u64> {
        let now = crate::stocks::chrono_now();
        let r = sqlx::query(
            "WITH todo AS (
               SELECT t.mint, t.creator, t.created_at, t.supply::float8 AS supply
               FROM tokens t JOIN token_stats ts ON ts.token_mint = t.mint
               WHERE t.source = 'stream' AND t.supply IS NOT NULL AND t.supply > 0
                 AND ts.last_trade_at > $1 - 3600
                 AND (ts.holders_at IS NULL OR ts.holders_at < $1 - CASE WHEN ts.last_trade_at > $1 - 300 THEN 120 ELSE 600 END)
               ORDER BY ts.holders_at NULLS FIRST, ts.last_trade_at DESC LIMIT $2),
             pos AS (
               SELECT tr.token_mint, tr.wallet,
                      SUM(CASE WHEN tr.side='buy' THEN tr.base_raw::float8 ELSE -tr.base_raw::float8 END) AS bal,
                      MIN(tr.block_time) FILTER (WHERE tr.side='buy') AS first_buy
               FROM trades tr JOIN todo ON todo.mint = tr.token_mint
               GROUP BY tr.token_mint, tr.wallet),
             held AS (SELECT * FROM pos WHERE bal > 0),
             ranked AS (SELECT token_mint, bal, ROW_NUMBER() OVER (PARTITION BY token_mint ORDER BY bal DESC) AS rn FROM held),
             agg AS (
               SELECT h.token_mint,
                      COUNT(*) AS holders,
                      SUM(CASE WHEN h.wallet = todo.creator THEN h.bal ELSE 0 END) AS dev_bal,
                      SUM(CASE WHEN h.first_buy IS NOT NULL AND h.first_buy <= todo.created_at + 10 THEN h.bal ELSE 0 END) AS sniper_bal
               FROM held h JOIN todo ON todo.mint = h.token_mint GROUP BY h.token_mint),
             top AS (SELECT token_mint, SUM(bal) AS top10 FROM ranked WHERE rn <= 10 GROUP BY token_mint)
             UPDATE token_stats ts SET
               holders = COALESCE(agg.holders, 0),
               top10_pct = LEAST(100, COALESCE(top.top10, 0) / todo.supply * 100),
               dev_pct = LEAST(100, COALESCE(agg.dev_bal, 0) / todo.supply * 100),
               snipers_pct = LEAST(100, COALESCE(agg.sniper_bal, 0) / todo.supply * 100),
               holders_at = $1
             FROM todo LEFT JOIN agg ON agg.token_mint = todo.mint LEFT JOIN top ON top.token_mint = todo.mint
             WHERE ts.token_mint = todo.mint")
            .bind(now).bind(max_tokens).execute(db).await?;
        Ok(r.rows_affected())
    }

    pub async fn save_cursor(&self, slot: u64, signature: Option<&str>) -> Result<()> {
        sqlx::query("INSERT INTO cursor (program, last_slot, last_signature, updated_at) VALUES ('stream',$1,$2,$3)
                     ON CONFLICT (program) DO UPDATE SET last_slot=$1, last_signature=$2, updated_at=$3")
            .bind(slot as i64).bind(signature).bind(crate::stocks::chrono_now()).execute(&self.db).await?;
        Ok(())
    }
    pub async fn load_cursor(&self) -> Result<Option<u64>> {
        let r: Option<(i64,)> = sqlx::query_as("SELECT last_slot FROM cursor WHERE program='stream'").fetch_optional(&self.db).await?;
        Ok(r.map(|x| x.0 as u64))
    }
}

/// Stats rollups on their own task, so a 5s query never blocks the trade stream (it did: one lag spike a minute).
/// fast: 15s · holders: 20s · 24h + per-floor: 60s. Runs never overlap because it's one sequential loop.
pub async fn rollup_loop(db: PgPool) {
    let (mut last_fast, mut last_holders, mut last_rollup) = (std::time::Instant::now(), std::time::Instant::now(), std::time::Instant::now());
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if last_fast.elapsed() > std::time::Duration::from_secs(15) { if let Err(e) = Store::rollup_fast(&db).await { tracing::warn!(%e, "rollup_fast") } last_fast = std::time::Instant::now(); }
        if last_holders.elapsed() > std::time::Duration::from_secs(20) { match Store::rollup_holders(&db, 40).await { Ok(n) => tracing::debug!(n, "holders"), Err(e) => tracing::warn!(%e, "rollup_holders") } last_holders = std::time::Instant::now(); }
        if last_rollup.elapsed() > std::time::Duration::from_secs(60) {
            let t = std::time::Instant::now();
            match Store::rollup(&db).await { Ok(n) => tracing::debug!(n, ms = t.elapsed().as_millis() as u64, "rollup"), Err(e) => tracing::warn!(%e, "rollup") }
            last_rollup = std::time::Instant::now();
        }
    }
}
