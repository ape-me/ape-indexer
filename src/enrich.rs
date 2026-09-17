//! Fills what events don't carry: token supply, on-chain metadata (name/symbol/uri) and the image from the uri JSON.
use anyhow::Result;
use bigdecimal::BigDecimal;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::str::FromStr;
use std::time::Duration;

const METAPLEX: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";

pub struct Rpc { c: reqwest::Client, url: String }

impl Rpc {
    pub fn new() -> Rpc { Rpc { c: reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap(), url: std::env::var("RPC_URL").expect("RPC_URL") } }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let r: Value = self.c.post(&self.url).json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})).send().await?.error_for_status()?.json().await?;
        if let Some(e) = r.get("error") { anyhow::bail!("rpc {method}: {e}"); }
        Ok(r["result"].clone())
    }
    pub async fn token_supply(&self, mint: &str) -> Result<(String, i16)> {
        let r = self.call("getTokenSupply", json!([mint])).await?;
        Ok((r["value"]["amount"].as_str().unwrap_or("0").to_string(), r["value"]["decimals"].as_i64().unwrap_or(6) as i16))
    }
    /// Metaplex metadata PDA: name, symbol, uri. Token-2022 metadata extension is handled by the uri fallback later.
    pub async fn metadata(&self, mint: &str) -> Result<Option<(String, String, String)>> {
        let pda = metadata_pda(mint)?;
        let r = self.call("getAccountInfo", json!([pda, {"encoding":"base64"}])).await?;
        let Some(data) = r["value"]["data"][0].as_str() else { return Ok(None) };
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)?;
        // Metadata: key u8, update_authority 32, mint 32, then borsh strings name, symbol, uri
        let mut rd = crate::borsh::Reader::new(&bytes);
        rd.skip(1 + 32 + 32)?;
        let name = rd.string()?; let symbol = rd.string()?; let uri = rd.string()?;
        Ok(Some((trim(&name), trim(&symbol), trim(&uri))))
    }
}

fn trim(s: &str) -> String { s.trim_end_matches('\0').trim().to_string() }

pub fn metadata_pda(mint: &str) -> Result<String> {
    let prog = bs58::decode(METAPLEX).into_vec()?; let mint_b = bs58::decode(mint).into_vec()?;
    for bump in (0..=255u8).rev() {
        let mut h = sha2::Sha256::new();
        use sha2::Digest;
        h.update(b"metadata"); h.update(&prog); h.update(&mint_b); h.update([bump]); h.update(&prog); h.update(b"ProgramDerivedAddress");
        let out = h.finalize();
        if !is_on_curve(&out) { return Ok(bs58::encode(out).into_string()); }
    }
    anyhow::bail!("no pda")
}

/// Ed25519 point decompression check (is the 32-byte value a valid curve point). PDA must be off-curve.
fn is_on_curve(b: &[u8]) -> bool { curve25519_check::is_on_curve(b) }

mod curve25519_check {
    // Minimal field arithmetic to test y-coordinate decompression, per RFC 8032 §5.1.3.
    use num_bigint::BigUint;
    use num_traits::{One, Zero};
    use std::sync::OnceLock;
    struct K { p: BigUint, d: BigUint }
    fn k() -> &'static K {
        static K_: OnceLock<K> = OnceLock::new();
        K_.get_or_init(|| {
            let p = (BigUint::one() << 255u32) - BigUint::from(19u32);
            let d = (&p - BigUint::from(121665u32)) * modinv(&BigUint::from(121666u32), &p) % &p;
            K { p, d }
        })
    }
    fn modinv(a: &BigUint, p: &BigUint) -> BigUint { a.modpow(&(p - BigUint::from(2u32)), p) }
    pub fn is_on_curve(b: &[u8]) -> bool {
        let K { p, d } = k();
        let mut y = b.to_vec(); let sign = y[31] >> 7; y[31] &= 0x7f;
        let y = BigUint::from_bytes_le(&y);
        if y >= *p { return false; }
        let y2 = &y * &y % p;
        let u = (&y2 + p - BigUint::one()) % p;
        let v = (d * &y2 + BigUint::one()) % p;
        let x2 = &u * modinv(&v, p) % p;
        if x2.is_zero() { return sign == 0; }
        // sqrt via x = x2^((p+3)/8), then check x^2 == x2 or -x2 (then multiply by 2^((p-1)/4))
        let mut x = x2.modpow(&((p + BigUint::from(3u32)) >> 3u32), p);
        if &x * &x % p != x2 {
            let i = BigUint::from(2u32).modpow(&((p - BigUint::one()) >> 2u32), p);
            x = x * i % p;
            if &x * &x % p != x2 { return false; }
        }
        true
    }
}

/// Parsed mint: decimals, supply, Token-2022 metadata and transfer fee if present.
pub struct MintInfo { pub decimals: i16, pub supply: String, pub meta: Option<(String, String, String)>, pub tax_bps: Option<i32> }

impl Rpc {
    pub async fn mint_info(&self, mint: &str) -> Result<Option<MintInfo>> {
        let r = self.call("getAccountInfo", json!([mint, {"encoding":"jsonParsed"}])).await?;
        let info = &r["value"]["data"]["parsed"]["info"];
        if info.is_null() { return Ok(None); }
        let mut meta = None; let mut tax_bps = None;
        for ext in info["extensions"].as_array().cloned().unwrap_or_default() {
            match ext["extension"].as_str() {
                Some("tokenMetadata") => { let st = &ext["state"]; meta = Some((trim(st["name"].as_str().unwrap_or("")), trim(st["symbol"].as_str().unwrap_or("")), trim(st["uri"].as_str().unwrap_or("")))); }
                Some("transferFeeConfig") => { tax_bps = ext["state"]["newerTransferFee"]["transferFeeBasisPoints"].as_i64().map(|b| b as i32); }
                _ => {}
            }
        }
        Ok(Some(MintInfo { decimals: info["decimals"].as_i64().unwrap_or(6) as i16, supply: info["supply"].as_str().unwrap_or("0").to_string(), meta, tax_bps }))
    }
}

/// One pass: up to `limit` tokens missing supply or name. Returns how many were touched.
pub async fn pass(db: &PgPool, rpc: &Rpc, http: &reqwest::Client, limit: i64) -> Result<usize> {
    let rows: Vec<(String, Option<String>, Option<String>, Option<BigDecimal>, Option<String>)> =
        sqlx::query_as("SELECT mint, name, uri, supply, image FROM tokens WHERE supply IS NULL OR name IS NULL OR (image IS NULL AND uri IS NOT NULL AND uri <> '') ORDER BY created_at DESC LIMIT $1")
        .bind(limit).fetch_all(db).await?;
    let mut n = 0;
    for (mint, name, uri, supply, image) in rows {
        let mut uri = uri;
        if supply.is_none() || name.is_none() {
            match rpc.mint_info(&mint).await {
                Ok(Some(mi)) => {
                    sqlx::query("UPDATE tokens SET supply=$2, decimals=$3, tax_bps=COALESCE($4, tax_bps) WHERE mint=$1")
                        .bind(&mint).bind(BigDecimal::from_str(&mi.supply)?).bind(mi.decimals).bind(mi.tax_bps).execute(db).await?;
                    let meta = match mi.meta { Some(m) => Some(m), None => rpc.metadata(&mint).await.unwrap_or_else(|e| { tracing::warn!(%e, mint, "metaplex"); None }) };
                    match meta {
                        Some((nm, sym, u)) => { sqlx::query("UPDATE tokens SET name=$2, symbol=$3, uri=$4 WHERE mint=$1 AND name IS NULL").bind(&mint).bind(&nm).bind(&sym).bind(&u).execute(db).await?; uri = Some(u); }
                        None => { sqlx::query("UPDATE tokens SET name='' WHERE mint=$1 AND name IS NULL").bind(&mint).execute(db).await?; }
                    }
                }
                Ok(None) => { tracing::warn!(mint, "mint account not found"); sqlx::query("UPDATE tokens SET name='' , supply=0 WHERE mint=$1 AND name IS NULL").bind(&mint).execute(db).await?; }
                Err(e) => tracing::warn!(%e, mint, "mint info"),
            }
        }
        if image.is_none() {
            if let Some(u) = uri.filter(|u| u.starts_with("http")) {
                let img = match http.get(&u).send().await.and_then(|r| r.error_for_status()) {
                    Ok(r) => r.json::<Value>().await.ok().and_then(|j| j["image"].as_str().map(String::from)),
                    Err(_) => None,
                };
                sqlx::query("UPDATE tokens SET image=$2 WHERE mint=$1").bind(&mint).bind(img.unwrap_or_default()).execute(db).await?;
            }
        }
        n += 1;
    }
    Ok(n)
}

pub async fn run(db: PgPool) {
    let rpc = Rpc::new();
    let http = reqwest::Client::builder().user_agent("Mozilla/5.0 ape-indexer").timeout(Duration::from_secs(10)).build().unwrap();
    loop {
        match pass(&db, &rpc, &http, 25).await {
            Ok(0) => tokio::time::sleep(Duration::from_secs(10)).await,
            Ok(n) => { tracing::info!(n, "enriched"); tokio::time::sleep(Duration::from_millis(500)).await }
            Err(e) => { tracing::warn!(%e, "enrich"); tokio::time::sleep(Duration::from_secs(10)).await }
        }
    }
}

