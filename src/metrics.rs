//! Prometheus metrics on 127.0.0.1:9464/metrics (scraped by the apme-ops Prometheus). Names are stable: the
//! Grafana dashboard in apme-ops depends on them.
use metrics::{counter, gauge, histogram};
use std::time::Duration;

pub fn install() {
    let addr: std::net::SocketAddr = std::env::var("METRICS_ADDR").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| "127.0.0.1:9464".parse().unwrap());
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .with_http_listener(addr)
        .set_buckets_for_metric(metrics_exporter_prometheus::Matcher::Suffix("_seconds".into()), &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]).unwrap()
        .install().expect("metrics exporter");
    tracing::info!(%addr, "metrics listening");
    describe();
}

fn describe() {
    use metrics::{describe_counter, describe_gauge, describe_histogram};
    describe_gauge!("ape_chain_slot", "newest confirmed slot seen on the Kaldera stream (BlockMeta)");
    describe_gauge!("ape_rpc_slot", "confirmed slot from the RPC, polled every 10s");
    describe_gauge!("ape_indexer_slot", "slot of the last transaction the indexer processed");
    describe_gauge!("ape_newest_trade_age_seconds", "now minus block_time of the newest stored trade");
    describe_gauge!("ape_pending_slots", "slots buffered waiting for their BlockMeta");
    describe_gauge!("ape_stream_connected", "1 while subscribed to the stream");
    describe_gauge!("ape_stocks", "stock mints in the subscription filter");
    describe_gauge!("ape_stock_prices_age_seconds", "seconds since stock USD prices were refreshed");
    describe_counter!("ape_tx_total", "transactions received from the stream");
    describe_counter!("ape_events_total", "decoded events by program and kind");
    describe_counter!("ape_trades_stored_total", "trades written to Postgres by program");
    describe_counter!("ape_trades_dup_total", "trades rejected as duplicates (replay)");
    describe_counter!("ape_reconnects_total", "stream reconnects");
    describe_counter!("ape_apply_errors_total", "failed writes that forced a replay");
    describe_counter!("ape_blocktime_extrapolated_total", "slots flushed without a BlockMeta");
    describe_counter!("ape_push_batches_total", "batches pushed to ape-be by result");
    describe_counter!("ape_push_trades_total", "trades pushed to ape-be");
    describe_counter!("ape_rpc_errors_total", "RPC call failures");
    describe_histogram!("ape_apply_seconds", "time to write one event to Postgres");
    describe_histogram!("ape_push_seconds", "time for one push to ape-be");
    describe_histogram!("ape_stream_delay_seconds", "now minus the stream message's created_at");
    describe_histogram!("ape_e2e_seconds", "now minus block_time when a trade is stored");
}

pub fn stream_connected(on: bool) { gauge!("ape_stream_connected").set(if on { 1.0 } else { 0.0 }); }
pub fn chain_slot(s: u64) { gauge!("ape_chain_slot").set(s as f64); }
pub fn indexer_slot(s: u64) { gauge!("ape_indexer_slot").set(s as f64); }
pub fn pending_slots(n: usize) { gauge!("ape_pending_slots").set(n as f64); }
pub fn stocks(n: usize) { gauge!("ape_stocks").set(n as f64); }
pub fn newest_trade_age(block_time: i64) { gauge!("ape_newest_trade_age_seconds").set((crate::stocks::chrono_now() - block_time).max(0) as f64); }
pub fn stock_prices_refreshed() { gauge!("ape_stock_prices_age_seconds").set(0.0); }
pub fn stock_prices_age(secs: f64) { gauge!("ape_stock_prices_age_seconds").set(secs); }
pub fn tx() { counter!("ape_tx_total").increment(1); }
pub fn event(program: &str, kind: &str) { counter!("ape_events_total", "program" => program.to_string(), "kind" => kind.to_string()).increment(1); }
pub fn trade_stored(program: &str) { counter!("ape_trades_stored_total", "program" => program.to_string()).increment(1); }
pub fn trade_dup() { counter!("ape_trades_dup_total").increment(1); }
pub fn reconnect() { counter!("ape_reconnects_total").increment(1); }
pub fn apply_error() { counter!("ape_apply_errors_total").increment(1); }
pub fn extrapolated() { counter!("ape_blocktime_extrapolated_total").increment(1); }
pub fn push(result: &'static str, trades: usize, took: Duration) {
    counter!("ape_push_batches_total", "result" => result).increment(1);
    if result == "ok" { counter!("ape_push_trades_total").increment(trades as u64); }
    histogram!("ape_push_seconds").record(took.as_secs_f64());
}
pub fn rpc_error() { counter!("ape_rpc_errors_total").increment(1); }
pub fn apply_took(d: Duration) { histogram!("ape_apply_seconds").record(d.as_secs_f64()); }
pub fn stream_delay(created_at: i64) { histogram!("ape_stream_delay_seconds").record((crate::stocks::chrono_now() - created_at).max(0) as f64); }
pub fn e2e(block_time: i64) { histogram!("ape_e2e_seconds").record((crate::stocks::chrono_now() - block_time).max(0) as f64); }

/// Polls the RPC's confirmed slot every 10s so a silent stream still shows as lag.
pub async fn rpc_slot_loop(rpc_url: String) {
    let c = reqwest::Client::builder().timeout(Duration::from_secs(5)).build().unwrap();
    loop {
        match c.post(&rpc_url).json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"getSlot","params":[{"commitment":"confirmed"}]})).send().await {
            Ok(r) => match r.json::<serde_json::Value>().await { Ok(v) => if let Some(s) = v["result"].as_u64() { gauge!("ape_rpc_slot").set(s as f64); }, Err(_) => rpc_error() },
            Err(_) => rpc_error(),
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}
