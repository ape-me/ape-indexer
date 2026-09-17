//! Raydium CPMM logs its SwapEvent with `emit!` (base64 in "Program data:" logs), not CPI.
//! Swap instructions (execution order) are paired with SwapEvent logs (log order), which match 1:1.
use super::{disc, orient, Ctx};
use crate::borsh::Reader;
use crate::events::{Event, Side};
use base64::Engine;

const INITIALIZE: [u8; 8] = hex!("afaf6d1f0d989bed");
const SWAP_BASE_INPUT: [u8; 8] = hex!("8fbe5adac41e33de");
const SWAP_BASE_OUTPUT: [u8; 8] = hex!("37d96256a34ab4ad");
const EV_SWAP: [u8; 8] = hex!("40c6cde8260871e2");

struct SwapLog { pool: String, in_before: u64, out_before: u64, in_amt: u64, out_amt: u64, in_fee: u64, out_fee: u64, in_mint: String, out_mint: String }

fn parse_log(bytes: &[u8]) -> Option<SwapLog> {
    let mut r = Reader::new(&bytes[8..]);
    let pool = r.pubkey().ok()?; let in_before = r.u64().ok()?; let out_before = r.u64().ok()?;
    let in_amt = r.u64().ok()?; let out_amt = r.u64().ok()?; let in_fee = r.u64().ok()?; let out_fee = r.u64().ok()?;
    let _base_input = r.bool().ok()?; let in_mint = r.pubkey().ok()?; let out_mint = r.pubkey().ok()?;
    Some(SwapLog { pool, in_before, out_before, in_amt, out_amt, in_fee, out_fee, in_mint, out_mint })
}

pub fn decode(ctx: &Ctx) -> Vec<Event> {
    let mut out = Vec::new();
    let pid = ctx.program.id();
    // Raydium CLMM emits a SwapEvent with the same discriminator, so attribute each
    // "Program data:" log to the program on top of the invoke stack.
    let mut stack: Vec<&str> = Vec::new();
    let mut logs: Vec<SwapLog> = Vec::new();
    for l in &ctx.tx.logs {
        if let Some(rest) = l.strip_prefix("Program ") {
            if let Some(i) = rest.find(" invoke [") { stack.push(&rest[..i]); continue; }
            if rest.ends_with(" success") || rest.contains(" failed") || rest.contains(" consumed ") && false { stack.pop(); continue; }
            if let Some(b64) = rest.strip_prefix("data: ") {
                if stack.last().copied() != Some(pid) { continue; }
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { continue };
                if bytes.len() < 16 || bytes[..8] != EV_SWAP { continue; }
                if let Some(s) = parse_log(&bytes) { logs.push(s); }
            }
        }
    }
    let mut li = 0usize;
    for (i, ix) in ctx.tx.ixs.iter().enumerate() {
        if ix.program != pid { continue; }
        let Some(d) = disc(ix) else { continue };
        let a = &ix.accounts;
        match d {
            INITIALIZE if a.len() > 11 => {
                let Some((base, quote)) = orient(&a[4], &a[5], ctx.stocks) else { continue };
                let (bv, qv) = if base == a[4] { (a[10].clone(), a[11].clone()) } else { (a[11].clone(), a[10].clone()) };
                out.push(Event::PoolCreated { meta: ctx.meta(i as u16), pool: a[3].clone(), base_mint: base, quote_mint: quote, creator: a[0].clone(), base_vault: Some(bv), quote_vault: Some(qv), token: None, holder_rewards: false });
            }
            SWAP_BASE_INPUT | SWAP_BASE_OUTPUT if a.len() > 11 => {
                let Some(l) = logs.get(li) else { continue }; li += 1;
                let Some((base, quote)) = orient(&l.in_mint, &l.out_mint, ctx.stocks) else { continue };
                let in_after = l.in_before + l.in_amt - l.in_fee; let out_after = l.out_before - l.out_amt;
                let (side, base_raw, quote_raw, rb, rq) = if l.in_mint == quote {
                    (Side::Buy, l.out_amt - l.out_fee, l.in_amt, out_after, in_after)
                } else {
                    (Side::Sell, l.in_amt, l.out_amt - l.out_fee, in_after, out_after)
                };
                out.push(Event::Swap { meta: ctx.meta(i as u16), pool: l.pool.clone(), base_mint: base, quote_mint: quote, wallet: a[0].clone(), side,
                    base_raw: base_raw as u128, quote_raw: quote_raw as u128, reserve_base_raw: Some(rb as u128), reserve_quote_raw: Some(rq as u128), sqrt_price_q64: None });
            }
            _ => {}
        }
    }
    out
}
