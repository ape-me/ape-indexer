//! Asset dictionary. Tokenized stocks come from their issuers (Backed, Backpack, PreStocks), crypto majors and
//! yield tokens from a hand-checked seed list. Everything prices and swaps the same way, so it all lives in `stocks`.
use anyhow::Result;
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const BACKED: &str = "https://api.backed.fi/api/v2/public/assets";
const BACKPACK_ASSETS: &str = "https://api.backpack.exchange/api/v1/assets";
const DEXSCREENER: &str = "https://api.dexscreener.com/tokens/v1/solana/";
const JUP_VERIFIED: &str = "https://lite-api.jup.ag/tokens/v2/tag?query=verified";
const ONDO_MINT_AUTHORITY: &str = "9foMHsSDq7nMg4WPusSz9eY7tyxyukqborA8GyU5cUxD";
/// Ondo names that mint and redeem around the clock; everything else is Sunday 8pm to Friday 8pm ET.
const ONDO_247: [&str; 31] = ["NVDA", "IBIT", "TSLA", "ORCL", "GOOGL", "MU", "DRAM", "STRC", "MSFT", "TSM", "AMZN", "MSTR", "SKHY", "CRCL", "TQQQ", "ETHA", "AAPL", "SNDK", "ARM", "SLV", "COIN", "MRVL", "AVGO", "META", "SPCX", "AMD", "GLD", "SPY", "INTC", "GLW", "QQQ"];
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

/// Also says whether every page came back: a partial list must not hide the xStocks it is missing.
async fn backed(c: &reqwest::Client) -> (Vec<Asset>, HashMap<String, String>, bool) {
    let (mut out, mut logos) = (Vec::new(), HashMap::new());
    for page in 1..=40 {
        let mut got = None;
        for attempt in 0..3 {
            if attempt > 0 { tokio::time::sleep(Duration::from_secs(2)).await }
            match c.get(format!("{BACKED}?page={page}")).send().await.and_then(|r| r.error_for_status()) {
                Ok(r) => match r.json::<BackedList>().await { Ok(v) => { got = Some(v); break } Err(e) => tracing::warn!(%e, page, "backed decode") },
                Err(e) => tracing::warn!(%e, page, attempt, "backed fetch"),
            }
        }
        let Some(r) = got else { return (out, logos, false) };
        let more = r.page.has_next;
        for a in r.nodes {
            let underlying = a.underlying.clone().unwrap_or_else(|| a.symbol.trim_end_matches('x').to_string());
            if let Some(l) = &a.logo { logos.insert(underlying.clone(), l.clone()); }
            let Some(d) = a.deployments.iter().find(|d| d.network == "Solana") else { continue };
            let name = a.name.trim_end_matches(" xStock").to_string();
            out.push(Asset { mint: d.address.clone(), category: stock_or_etf(&name, &underlying), symbol: a.symbol, name, issuer: "xstocks", underlying, logo: a.logo, halted: a.halted.unwrap_or(false), tags: vec![] });
        }
        if !more { break }
    }
    tracing::info!(n = out.len(), "backed assets");
    (out, logos, true)
}

/// iOS cannot decode SVG, and some token metadata points at HTML or text. Backpack serves SVG for most tickers, so
/// each Backpack and PreStocks logo is checked and falls back to Backed's PNG, then Ondo's, for the same ticker.
async fn is_bitmap(c: &reqwest::Client, url: &str) -> bool {
    let Ok(r) = c.head(url).send().await else { return false };
    r.status().is_success() && r.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("image/") && !t.contains("svg"))
}

async fn bitmap_logo(c: &reqwest::Client, a: &Asset, backed: &HashMap<String, String>) -> Option<String> {
    let ondo = format!("https://cdn.ondo.finance/tokens/logos/{}on_160x160.png", a.underlying.to_lowercase());
    for url in a.logo.iter().chain(backed.get(&a.underlying)).chain(std::iter::once(&ondo)) {
        if is_bitmap(c, url).await { return Some(url.clone()) }
    }
    tracing::warn!(symbol = %a.symbol, "no bitmap logo");
    None
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

// ── Ondo: their own feed sits behind a browser challenge, but every Ondo mint is on Jupiter's verified list
// under one mint authority, with a PNG logo on Ondo's CDN. Trades go through Jupiter's RFQ, not pools. ──
#[derive(Deserialize)]
struct JupToken {
    id: String, name: String, symbol: String, icon: Option<String>,
    #[serde(rename = "mintAuthority")] mint_authority: Option<String>,
    mcap: Option<f64>, liquidity: Option<f64>, #[serde(default)] tags: Option<Vec<String>>,
}

async fn jup_verified(c: &reqwest::Client) -> Vec<JupToken> {
    match c.get(JUP_VERIFIED).send().await.and_then(|r| r.error_for_status()) {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => { tracing::warn!(%e, "jupiter verified list"); vec![] }
    }
}

fn ondo(list: &[JupToken]) -> Vec<Asset> {
    let out: Vec<Asset> = list.iter().filter(|t| t.mint_authority.as_deref() == Some(ONDO_MINT_AUTHORITY)).filter_map(|t| {
        let under = t.symbol.strip_suffix("on")?.to_string();
        let name = t.name.trim_end_matches(" (Ondo Tokenized)").to_string();
        let tags = if ONDO_247.contains(&under.as_str()) { vec!["247".to_string()] } else { vec![] };
        Some(Asset { mint: t.id.clone(), category: stock_or_etf(&name, &under), symbol: t.symbol.clone(), name, issuer: "ondo", underlying: under, logo: t.icon.clone(), halted: false, tags })
    }).collect();
    tracing::info!(n = out.len(), "ondo assets");
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
// for half of these, so the mint is the source of truth; logos are PNGs (Jupiter serves SVGs iOS cannot decode).
// (mint, symbol, name, category, group, underlying, logo) ──
const SEEDS: [(&str, &str, &str, &str, &str, &str, &str); 31] = [
    ("So11111111111111111111111111111111111111112", "SOL", "Solana", "crypto", "majors", "SOL", "https://coin-images.coingecko.com/coins/images/4128/large/solana.png"),
    ("3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh", "WBTC", "Wrapped BTC (Portal)", "crypto", "majors", "BTC", "https://coin-images.coingecko.com/coins/images/7598/large/WBTCLOGO.png"),
    ("cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij", "cbBTC", "Coinbase Wrapped BTC", "crypto", "majors", "BTC", "https://coin-images.coingecko.com/coins/images/40143/large/cbbtc.webp"),
    ("7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs", "ETH", "Ether (Portal)", "crypto", "majors", "ETH", "https://coin-images.coingecko.com/coins/images/279/large/ethereum.png"),
    ("6UpQcMAb5xMzxc7ZfPaVMgx3KqsvKZdT5U718BzD5We2", "wXRP", "Wrapped XRP", "crypto", "majors", "XRP", "https://coin-images.coingecko.com/coins/images/44/large/xrp-symbol-white-128.png"),
    ("9gP2kCy3wA1ctvYWQk75guqXuHfrEomqydHLtcTCqiLa", "BNB", "Binance Coin (Portal)", "crypto", "majors", "BNB", "https://coin-images.coingecko.com/coins/images/825/large/bnb-icon2_2x.png"),
    ("DoGEV7LASBkQbibMc5k5vKnTZoMg423GpJ5QtJEGfm7R", "DOGE", "Dogecoin", "crypto", "majors", "DOGE", "https://coin-images.coingecko.com/coins/images/5/large/dogecoin.png"),
    ("cbLTC4T5NpzSUtQ7ekgEMZGaUPVJY1ko6BUikqa4gGf", "cbLTC", "Coinbase Wrapped LTC", "crypto", "majors", "LTC", "https://coin-images.coingecko.com/coins/images/2/large/litecoin.png"),
    ("avaxGHCq3T7hoxd73oY2KY9hJSTaeMibXvHy5KNzh5D", "AVAX", "Avalanche", "crypto", "l1", "AVAX", "https://coin-images.coingecko.com/coins/images/12559/large/Avalanche_Circle_RedWhite_Trans.png"),
    ("suifhC9gU1VbJAPYPTBkHJyyyStKGLLYPVDTmPoqbvA", "SUI", "Sui", "crypto", "l1", "SUI", "https://coin-images.coingecko.com/coins/images/26375/large/sui-ocean-square.png"),
    ("3ZLekZYq2qkZiSpnSvabjit34tUkjSwD1JFuW9as9wBG", "wNEAR", "Wrapped NEAR", "crypto", "l1", "NEAR", "https://coin-images.coingecko.com/coins/images/10365/large/near.jpg"),
    ("98sMhvDwXj1RQi5c5Mndm3vPe9cBqPrbLaufMXFNMh5g", "HYPE", "Hyperliquid", "crypto", "l1", "HYPE", "https://coin-images.coingecko.com/coins/images/50882/large/hyperliquid.jpg"),
    ("taoC6xyv2v8tDLcev4uaGUgV4vdQsWJrGft2kcBRrBY", "TAO", "Bittensor", "crypto", "l1", "TAO", "https://coin-images.coingecko.com/coins/images/28452/large/ARUsPeNQ_400x400.jpeg"),
    ("ARBzQTYDCW2KnVEjs1Mc81LekB1ibVFZKbSVmorkoT9d", "ARB", "Arbitrum", "crypto", "l1", "ARB", "https://coin-images.coingecko.com/coins/images/16547/large/arb.jpg"),
    ("LinkhB3afbBKb2EQQu7s7umdZceV3wcvAUJhQAfQ23L", "LINK", "Chainlink", "crypto", "defi", "LINK", "https://coin-images.coingecko.com/coins/images/877/large/Chainlink_Logo_500.png"),
    ("uniHfuPhEQSrtpzXpJZDCSq53yaejKKpNhFUiKoHKHV", "UNI", "Uniswap", "crypto", "defi", "UNI", "https://coin-images.coingecko.com/coins/images/12504/large/uniswap-logo.png"),
    ("AavE1kKKnesPw4MuRJmJ9jZs9QzEE8CPxQ3ViczUDfc1", "AAVE", "Aave", "crypto", "defi", "AAVE", "https://coin-images.coingecko.com/coins/images/12645/large/aave-token-round.png"),
    ("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R", "RAY", "Raydium", "crypto", "defi", "RAY", "https://coin-images.coingecko.com/coins/images/13928/large/PSigc4ie_400x400.jpg"),
    ("JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN", "JUP", "Jupiter", "crypto", "defi", "JUP", "https://coin-images.coingecko.com/coins/images/34188/large/jup.png"),
    ("jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL", "JTO", "Jito", "crypto", "defi", "JTO", "https://coin-images.coingecko.com/coins/images/33228/large/jto.png"),
    ("HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3", "PYTH", "Pyth Network", "crypto", "defi", "PYTH", "https://coin-images.coingecko.com/coins/images/31924/large/pyth.png"),
    ("PEPEqnuuCDbBC89p1u9vpnP1KQ2oj1xTcQBsjt9X55m", "PEPE", "Pepe", "crypto", "memes", "PEPE", "https://coin-images.coingecko.com/coins/images/29850/large/pepe-token.jpeg"),
    ("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263", "BONK", "Bonk", "crypto", "memes", "BONK", "https://coin-images.coingecko.com/coins/images/28600/large/bonk.jpg"),
    ("EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm", "WIF", "dogwifhat", "crypto", "memes", "WIF", "https://coin-images.coingecko.com/coins/images/33566/large/dogwifhat.jpg"),
    ("2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv", "PENGU", "Pudgy Penguins", "crypto", "memes", "PENGU", "https://coin-images.coingecko.com/coins/images/52622/large/PUDGY_PENGUINS_PENGU_PFP.png"),
    ("6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN", "TRUMP", "Official Trump", "crypto", "memes", "TRUMP", "https://coin-images.coingecko.com/coins/images/53746/large/trump.png"),
    ("rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBof", "RENDER", "Render", "crypto", "solana", "RENDER", "https://coin-images.coingecko.com/coins/images/11636/large/rndr.png"),
    ("hntyVP6YFm1Hg25TN9WGLqM12b8TQmcknKrdu1oxWux", "HNT", "Helium", "crypto", "solana", "HNT", "https://coin-images.coingecko.com/coins/images/4284/large/helium_logo_use.png"),
    ("AvZZF1YaZDziPY2RCK4oJrRVrbN3mTD9NL24hPeaZeUj", "syrupUSDC", "Maple Syrup USDC", "earn", "earn", "syrupUSDC", "https://coin-images.coingecko.com/coins/images/54658/large/syrupUSDC.png"),
    ("DEkqHyPN7GMRJ5cArtQFAWefqbZb33Hyf6s5iCwjEonT", "USDe", "Ethena USDe", "earn", "earn", "USDe", "https://coin-images.coingecko.com/coins/images/33613/large/usde.png"),
    ("A1KLoBrKBde8Ty9qtNQUtq3C2ortoC3u7twggz7sEto6", "USDY", "Ondo US Dollar Yield", "earn", "earn", "USDY", "https://coin-images.coingecko.com/coins/images/31700/large/usdy_%281%29.png"),
];

// ── Solana's own coins: Jupiter-verified, ranked by market cap. In at the top 50, out below 75, so the shelf does not
// churn day to day. Stablecoins, staked SOL, yield and stock tokens, and anything already listed are left out. ──
const TOP_COINS: usize = 50;
const TOP_COINS_KEEP: usize = 75;
const TOP_COIN_MIN_POOL_USD: f64 = 500_000.0;
const NOT_COINS: [&str; 14] = ["stable", "lst", "yield", "yb", "rwa", "stocks", "xstocks", "ondo", "equities", "backpack", "prestocks", "jup-lend-earn", "duplicate", "commodities"];

// A coin never takes a mint or symbol another coin has, nor is it a wrapped BTC/ETH/SOL. It may share a stock's ticker
// (PUMP, MET) unless it is a memecoin, which is how the "GME" impersonators look.
async fn top_coins(c: &reqwest::Client, list: &[JupToken], listed: &HashSet<String>, coins: &HashSet<String>, tickers: &HashSet<String>) -> Vec<Asset> {
    let under = |t: &JupToken| t.symbol.trim_start_matches('$').to_uppercase();
    let mut ranked: Vec<&JupToken> = list.iter().filter(|t| {
        let tags = t.tags.as_deref().unwrap_or_default();
        let u = under(t);
        t.liquidity.unwrap_or(0.0) >= TOP_COIN_MIN_POOL_USD && t.mcap.unwrap_or(0.0) > 0.0
            && !tags.iter().any(|g| NOT_COINS.contains(&g.as_str()))
            && !u.contains("USD") && !u.contains("EUR")
            && !(u.len() > 3 && ["BTC", "ETH", "SOL"].iter().any(|x| u.ends_with(x)))
            && !coins.contains(&u) && !coins.contains(&t.id)
            && !(tickers.contains(&u) && tags.iter().any(|g| g == "meme"))
    }).collect();
    ranked.sort_by(|a, b| b.mcap.unwrap_or(0.0).total_cmp(&a.mcap.unwrap_or(0.0)));
    let picked: Vec<Asset> = ranked.into_iter().enumerate()
        .filter(|(i, t)| *i < TOP_COINS || (*i < TOP_COINS_KEEP && listed.contains(&t.id)))
        .map(|(_, t)| {
            let tags = t.tags.as_deref().unwrap_or_default();
            let group = if tags.iter().any(|g| g == "meme") { "memes" } else if tags.iter().any(|g| g == "defi") { "defi" } else { "solana" };
            Asset { mint: t.id.clone(), symbol: t.symbol.clone(), name: t.name.clone(), issuer: "crypto", category: "crypto", underlying: under(t),
                    logo: t.icon.clone(), halted: false, tags: vec![group.to_string(), "top".to_string()] }
        }).collect();
    // Jupiter serves some icons as SVG, which iOS cannot draw: those coins go without a logo.
    let out: Vec<Asset> = futures::stream::iter(picked).map(|mut a| async move {
        if let Some(u) = a.logo.take() { if is_bitmap(c, &u).await { a.logo = Some(u) } }
        a
    }).buffer_unordered(8).collect().await;
    tracing::info!(n = out.len(), "top solana coins");
    out
}

fn seeds() -> Vec<Asset> {
    SEEDS.iter().map(|&(mint, symbol, name, category, group, underlying, logo)| Asset {
        mint: mint.into(), symbol: symbol.into(), name: name.into(), issuer: if category == "earn" { "earn" } else { "crypto" },
        category, underlying: underlying.into(), logo: Some(logo.into()), halted: false, tags: vec![group.to_string()],
    }).collect()
}

/// Upsert the whole catalog. Returns the tokenized-stock mints the stream watches as quote tokens: crypto and
/// yield tokens are deliberately left out, SOL alone would subscribe the stream to the entire chain.
pub async fn sync_list(db: &PgPool) -> Result<HashSet<String>> {
    let c = client();
    let marks = prestocks_marks(&c).await;
    let (mut assets, backed_logos, backed_ok) = backed(&c).await;
    // Only rows that can reach the shelf get the HEAD round-trips: Backpack lists 1,100+ mints, a few dozen are liquid.
    let liquid: HashSet<String> = sqlx::query_scalar("SELECT mint FROM stocks WHERE coalesce(liquidity_usd, 0) >= 1000")
        .fetch_all(db).await?.into_iter().collect();
    let (c, backed_logos, liquid) = (&c, &backed_logos, &liquid);
    let backpack = backpack(c).await;
    // A source that failed this run keeps its rows as they are instead of hiding them all.
    let mut failed: Vec<&str> = vec![];
    if !backed_ok { failed.push("xstocks") }
    if backpack.is_empty() { failed.push("backpack") }
    if marks.is_empty() { failed.push("prestocks") }
    let checked: Vec<Asset> = futures::stream::iter(backpack.into_iter().chain(prestocks(&marks)))
        .map(|mut a| async move { if liquid.contains(&a.mint) { a.logo = bitmap_logo(c, &a, backed_logos).await } a })
        .buffer_unordered(8).collect().await;
    assets.extend(checked);
    let jup = jup_verified(c).await;
    let ondo = ondo(&jup);
    if ondo.is_empty() { failed.push("ondo") }
    assets.extend(ondo);
    assets.extend(seeds());
    let is_coin = |a: &&Asset| matches!(a.category, "crypto" | "earn");
    let coins: HashSet<String> = assets.iter().map(|a| a.mint.clone())
        .chain(assets.iter().filter(is_coin).map(|a| a.underlying.to_uppercase())).collect();
    let tickers: HashSet<String> = assets.iter().filter(|a| !is_coin(a)).map(|a| a.underlying.to_uppercase()).collect();
    let listed: HashSet<String> = sqlx::query_scalar("SELECT mint FROM stocks WHERE NOT excluded AND 'top' = ANY(tags)")
        .fetch_all(db).await?.into_iter().collect();
    let top = top_coins(c, &jup, &listed, &coins, &tickers).await;
    if top.is_empty() { failed.push("crypto") }
    assets.extend(top);
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
                     ON CONFLICT (mint) DO UPDATE SET symbol=EXCLUDED.symbol, name=EXCLUDED.name, issuer=EXCLUDED.issuer, category=EXCLUDED.category, logo=EXCLUDED.logo, excluded=EXCLUDED.excluded, tags=EXCLUDED.tags, halted=EXCLUDED.halted, underlying=EXCLUDED.underlying")
            .bind(&a.mint).bind(&a.symbol).bind(&a.name).bind(a.issuer).bind(&cat).bind(&a.logo).bind(now).bind(excluded).bind(&tags).bind(a.halted).bind(&a.underlying)
            .execute(db).await?;
        if !excluded && matches!(a.issuer, "xstocks" | "backpack" | "prestocks") { watched.insert(a.mint); }
    }
    // A row no issuer lists any more is hidden, not deleted: trades and orders still reference it.
    let synced: Vec<&str> = assets_mints.iter().map(String::as_str).collect();
    sqlx::query("UPDATE stocks SET excluded = true WHERE NOT excluded AND NOT (mint = ANY($1)) AND NOT (issuer = ANY($2))").bind(&synced).bind(&failed).execute(db).await?;
    if !failed.is_empty() { tracing::warn!(?failed, "catalog sources failed; their rows kept as they were") }
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
async fn jupiter_prices(c: &reqwest::Client, mints: &[String]) -> HashMap<String, JupPrice> {
    let mut out = HashMap::new();
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

/// Our own median price per mint over the hour around `secs` ago: one junk tick on a thin pool can't move it. 24h and
/// 1h change come from this, not from DexScreener's single-pool figure, which once printed +4,876,972% on SK hynix.
async fn prices_ago(db: &PgPool, now: i64, secs: i64) -> HashMap<String, f64> {
    sqlx::query_as::<_, (String, f64)>("SELECT mint, percentile_cont(0.5) WITHIN GROUP (ORDER BY price_usd) FROM stock_snapshots WHERE ts > $1 - 1800 AND ts <= $1 + 1800 AND price_usd > 0 GROUP BY mint")
        .bind(now - secs).fetch_all(db).await.unwrap_or_default().into_iter().collect()
}
fn pct(now: f64, then: Option<&f64>) -> Option<f64> { then.map(|t| (now / t - 1.0) * 100.0) }

/// One pass of market data: 30 mints per DexScreener call, best pair by liquidity, plus PreStocks marks.
/// Updates the live columns on `stocks`; with `snapshot` also appends a row per stock to `stock_snapshots`.
/// `all` sweeps the whole catalog so a mint that just got its first pool is found; otherwise only pooled mints.
pub async fn refresh_prices(db: &PgPool, snapshot: bool, all: bool) -> Result<usize> {
    let mints: Vec<String> = if all { sqlx::query_scalar("SELECT mint FROM stocks WHERE issuer <> 'ondo'").fetch_all(db).await? }
        else { sqlx::query_scalar("SELECT mint FROM stocks WHERE coalesce(liquidity_usd, 0) > 0 AND issuer <> 'ondo'").fetch_all(db).await? };
    let c = client(); let now = chrono_now(); let mut updated = 0;
    let marks = prestocks_marks(&c).await;
    // The catalog sweep only needs to learn which mints have a pool, and DexScreener says that. Jupiter is
    // rate-limited at ~60 calls a minute and the tick loop already spends most of it; the sweep stays off it.
    let jup = if all { HashMap::new() } else { jupiter_prices(&c, &mints).await };
    let (p24, p1) = (prices_ago(db, now, 86400).await, prices_ago(db, now, 3600).await);
    let mult: HashMap<String, f64> = sqlx::query_as::<_, (String, f64)>("SELECT mint, multiplier FROM stocks WHERE multiplier > 0").fetch_all(db).await?.into_iter().collect();
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
            // price: Jupiter first (all pools), DexScreener's best pool as fallback. DexScreener prices a raw token, which
            // is `multiplier` shares after a split (NFLXx = 10), so it is brought back to one share.
            let dex = p["priceUsd"].as_str().and_then(|s| s.parse::<f64>().ok()).map(|x| x / mult.get(mint).copied().unwrap_or(1.0));
            let Some(price) = j.map(|j| j.usd).or(dex) else { continue };
            let liq = match (j.and_then(|j| j.liquidity), f(&p["liquidity"]["usd"])) { (Some(a), Some(b)) => Some(a.max(b)), (a, b) => a.or(b) };
            let (v24, v1, b24, s24, mcap) = (f(&p["volume"]["h24"]), f(&p["volume"]["h1"]), i(&p["txns"]["h24"]["buys"]), i(&p["txns"]["h24"]["sells"]), f(&p["marketCap"]).or(f(&p["fdv"])));
            let (change, change_1h) = (pct(price, p24.get(mint)), pct(price, p1.get(mint)));
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

/// Ondo names fill through Jupiter's RFQ at the real share price, so DexScreener and the trade stream never see them,
/// and Jupiter's own last price for them goes stale (MSFTon sat 32% over Microsoft for a day). The price shown is the
/// underlying's, which is what the RFQ fills at; Jupiter's figure is the fallback when it has none. One pass = 9 calls.
pub async fn refresh_ondo(db: &PgPool, c: &reqwest::Client, snapshot: bool) -> Result<usize> {
    let mints: Vec<String> = sqlx::query_scalar("SELECT mint FROM stocks WHERE issuer = 'ondo' AND NOT excluded").fetch_all(db).await?;
    if mints.is_empty() { return Ok(0) }
    let jup = jupiter_prices(c, &mints).await;
    let now = chrono_now();
    let p24 = prices_ago(db, now, 86400).await;
    for (mint, j) in &jup {
        let price = j.underlying.filter(|m| *m > 0.0).unwrap_or(j.usd);
        sqlx::query("UPDATE stocks SET price_usd=$2, change_24h=$3, updated_at=$4, liquidity_usd=$5, mark_usd=$6, premium_pct=NULL, mark_valuation=$7 WHERE mint=$1")
            .bind(mint).bind(price).bind(pct(price, p24.get(mint))).bind(now).bind(j.liquidity).bind(j.underlying).bind(j.underlying_mcap).execute(db).await?;
        sqlx::query("INSERT INTO stock_ticks (mint, ts, price_usd) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING").bind(mint).bind(now).bind(price).execute(db).await?;
        if snapshot {
            sqlx::query("INSERT INTO stock_snapshots (mint, ts, price_usd, liquidity_usd, mark_usd, mark_valuation) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
                .bind(mint).bind(now).bind(price).bind(j.liquidity).bind(j.underlying).bind(j.underlying_mcap).execute(db).await?;
        }
    }
    Ok(jup.len())
}

/// Every 30s: live columns for pooled mints. Every other pass (60s): a snapshot row and the Ondo pass. Once an
/// hour: the whole catalog, for pool discovery, and drop snapshots older than 30 days.
pub async fn price_loop(db: PgPool) {
    let c = client();
    let mut pass: u64 = 0;
    loop {
        let last = std::time::Instant::now();
        match refresh_prices(&db, pass % 2 == 0, pass % 120 == 0).await { Ok(n) => { crate::metrics::stock_prices_refreshed(); tracing::info!(n, "stock prices refreshed") }, Err(e) => tracing::warn!(%e, "price loop") }
        if pass % 2 == 0 {
            match refresh_ondo(&db, &c, pass % 4 == 0).await { Ok(n) => tracing::info!(n, "ondo prices refreshed"), Err(e) => tracing::warn!(%e, "ondo prices") }
        }
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
        let rows: Vec<(String, Option<f64>, Option<f64>)> = match sqlx::query_as("SELECT mint, mark_usd, change_24h FROM stocks WHERE NOT excluded AND coalesce(liquidity_usd, 0) >= 1000 AND issuer <> 'ondo'").fetch_all(&db).await { Ok(r) => r, Err(e) => { tracing::warn!(%e, "ticks: stocks"); continue } };
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
