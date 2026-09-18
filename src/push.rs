//! Pushes trades to ape-be: batched every 250ms, HMAC-SHA256 signed, three tries then dropped.
//! The Worker only fans these out to open WebSockets; Postgres stays the source of truth, so a
//! dropped batch costs a live update, never data.
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IngestTrade {
    pub mint: String, pub pool: String, pub program: crate::events::Program,
    pub sig: String, pub ts: i64, pub slot: u64, pub side: crate::events::Side, pub wallet: String,
    pub base: f64, pub quote: f64, pub price_quote: f64, pub price_usd: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Batch<'a> { trades: &'a [IngestTrade], sent_at: i64 }

pub struct Pusher { tx: mpsc::UnboundedSender<IngestTrade> }

impl Pusher {
    /// `url` is the full ingest endpoint, e.g. https://apme-be.iamjoey.workers.dev/ingest/trades
    pub fn start(url: String, secret: String) -> Pusher {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(url, secret, rx));
        Pusher { tx }
    }
    pub fn send(&self, t: IngestTrade) { let _ = self.tx.send(t); }
}

fn sign(secret: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(body.as_bytes());
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

async fn run(url: String, secret: String, mut rx: mpsc::UnboundedReceiver<IngestTrade>) {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(5)).build().expect("client");
    let mut buf: Vec<IngestTrade> = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            m = rx.recv() => match m { Some(t) => { buf.push(t); if buf.len() < 1000 { continue } } None => return },
            _ = tick.tick() => {}
        }
        if buf.is_empty() { continue }
        let trades: Vec<IngestTrade> = buf.drain(..).collect();
        let body = serde_json::to_string(&Batch { trades: &trades, sent_at: crate::stocks::chrono_now() }).expect("json");
        let sig = sign(&secret, &body);
        let mut ok = false; let t0 = std::time::Instant::now();
        for attempt in 0..3u32 {
            match client.post(&url).header("content-type", "application/json").header("x-signature", &sig).body(body.clone()).send().await {
                Ok(r) if r.status().is_success() => { ok = true; break }
                Ok(r) => { crate::metrics::push("failed", 0, t0.elapsed()); tracing::warn!(status = %r.status(), attempt, "push rejected") }
                Err(e) => { crate::metrics::push("failed", 0, t0.elapsed()); tracing::warn!(%e, attempt, "push failed") }
            }
            tokio::time::sleep(Duration::from_millis(200 * (attempt as u64 + 1))).await;
        }
        crate::metrics::push(if ok { "ok" } else { "dropped" }, trades.len(), t0.elapsed());
        if ok { tracing::debug!(n = trades.len(), "pushed") } else { tracing::error!(n = trades.len(), "push dropped") }
    }
}
