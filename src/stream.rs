//! Kaldera (Yellowstone) stream: six program filters, each requiring the program and any stock mint.
use crate::decode;
use crate::events::Program;
use crate::store::Store;
use crate::tx::TxView;
use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest, SubscribeRequestFilterBlocksMeta, SubscribeRequestFilterTransactions, SubscribeRequestPing};

fn request(stocks: Vec<String>, from_slot: Option<u64>) -> SubscribeRequest {
    let mut transactions = HashMap::new();
    for p in Program::ALL {
        transactions.insert(p.name().to_string(), SubscribeRequestFilterTransactions {
            vote: Some(false), failed: Some(false), signature: None,
            account_include: stocks.clone(), account_exclude: vec![], account_required: vec![p.id().to_string()],
        });
    }
    let mut blocks_meta = HashMap::new();
    blocks_meta.insert("meta".to_string(), SubscribeRequestFilterBlocksMeta {});
    SubscribeRequest { transactions, blocks_meta, commitment: Some(CommitmentLevel::Confirmed as i32), from_slot, ..Default::default() }
}

pub async fn run(mut store: Store) -> Result<()> {
    let url = std::env::var("GRPC_URL")?; let token = std::env::var("API_KEY").ok();
    let mut from_slot = store.load_cursor().await?;
    if let Some(s) = from_slot { tracing::info!(slot = s, "resuming from cursor"); }
    loop {
        match run_once(&mut store, &url, token.as_deref(), from_slot).await {
            Ok(last) => { tracing::warn!("stream ended, reconnecting"); from_slot = last; }
            Err(e) => { tracing::warn!(%e, "stream error, reconnecting in 2s"); from_slot = store.load_cursor().await.ok().flatten(); tokio::time::sleep(Duration::from_secs(2)).await; }
        }
    }
}

async fn run_once(store: &mut Store, url: &str, token: Option<&str>, from_slot: Option<u64>) -> Result<Option<u64>> {
    let mut client = GeyserGrpcClient::build_from_shared(url.to_string())?
        .x_token(token.map(|t| t.to_string()))?
        .tls_config(ClientTlsConfig::new().with_native_roots())?
        .max_decoding_message_size(64 * 1024 * 1024)
        .connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30))
        .connect().await.context("grpc connect")?;
    let stocks: Vec<String> = store.stock_set().into_iter().collect();
    let (mut sink, mut stream) = client.subscribe_with_request(Some(request(stocks, from_slot))).await.context("subscribe")?;
    tracing::info!(from_slot, "subscribed");
    let mut last_slot: Option<u64> = None; let mut last_sig: Option<String> = None;
    let mut block_times: HashMap<u64, i64> = HashMap::new();
    let mut last_meta: Option<(u64, i64)> = None;
    let mut n_stocks = store.stocks.len();
    let mut last_stock_check = Instant::now();
    let mut last_cursor = Instant::now(); let mut last_reload = Instant::now(); let mut last_rollup = Instant::now();
    let mut n_tx = 0u64; let mut n_ev = 0u64; let mut last_log = Instant::now();
    while let Some(msg) = stream.next().await {
        let msg = msg.context("stream recv")?;
        let created = msg.created_at.as_ref().map(|t| t.seconds).unwrap_or_else(crate::stocks::chrono_now);
        match msg.update_oneof {
            Some(UpdateOneof::Transaction(tx)) => {
                n_tx += 1;
                // block time from the stream's BlockMeta for this slot; if it hasn't arrived yet, extrapolate from the newest known slot (400ms/slot)
                let bt = match block_times.get(&tx.slot) {
                    Some(t) => *t,
                    None => match last_meta { Some((s, t)) => t + ((tx.slot as i64 - s as i64) * 2) / 5, None => created },
                };
                let view = match TxView::from_geyser(&tx, bt) { Ok(v) => v, Err(e) => { tracing::warn!(%e, "tx view"); continue } };
                let stocks = store.stock_set();
                for ev in decode::decode(&view, &stocks) {
                    match store.apply(&ev).await {
                        Ok(true) => n_ev += 1,
                        Ok(false) => {}
                        // a failed write must not be skipped: drop the connection and replay from the last saved cursor
                        Err(e) => { tracing::error!(%e, sig = %view.signature, "apply failed, replaying from cursor"); anyhow::bail!("apply: {e}"); }
                    }
                }
                last_slot = Some(tx.slot); last_sig = Some(view.signature);
            }
            Some(UpdateOneof::BlockMeta(m)) => {
                if let Some(t) = m.block_time.as_ref().map(|t| t.timestamp) {
                    block_times.insert(m.slot, t); last_meta = Some((m.slot, t));
                    if block_times.len() > 8192 { let cut = m.slot.saturating_sub(4096); block_times.retain(|k, _| *k >= cut); }
                }
            }
            Some(UpdateOneof::Ping(_)) => { sink.send(SubscribeRequest { ping: Some(SubscribeRequestPing { id: 1 }), ..Default::default() }).await.ok(); }
            _ => {}
        }
        if last_cursor.elapsed() > Duration::from_secs(5) { if let Some(s) = last_slot { store.save_cursor(s, last_sig.as_deref()).await.ok(); } last_cursor = Instant::now(); }
        if last_reload.elapsed() > Duration::from_secs(60) { store.reload().await.ok(); last_reload = Instant::now(); }
        if last_stock_check.elapsed() > Duration::from_secs(600) {
            last_stock_check = Instant::now();
            if let Ok(set) = crate::stocks::sync_list(&store.db).await { let _ = store.reload().await;
                if set.len() != n_stocks { n_stocks = set.len(); tracing::info!(n = n_stocks, "stock list changed, resubscribing"); sink.send(request(set.into_iter().collect(), None)).await.ok(); }
            }
        }
        if last_rollup.elapsed() > Duration::from_secs(60) { match store.rollup().await { Ok(n) => tracing::debug!(n, "rollup"), Err(e) => tracing::warn!(%e, "rollup") } last_rollup = Instant::now(); }
        if last_log.elapsed() > Duration::from_secs(30) { tracing::info!(n_tx, n_ev, slot = last_slot, "stream"); n_tx = 0; n_ev = 0; last_log = Instant::now(); }
    }
    Ok(last_slot)
}
