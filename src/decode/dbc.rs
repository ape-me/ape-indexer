use super::{disc, event_disc, event_payload, pair, Ctx};
use crate::borsh::Reader;
use crate::events::{Event, Side};
use crate::tx::Ix;

const INIT_SPL: [u8; 8] = hex!("8c55d7b06636684f");
const INIT_T22: [u8; 8] = hex!("a976334e916edc9b");
const SWAP: [u8; 8] = hex!("f8c69e91e17587c8");
const SWAP2: [u8; 8] = hex!("414b3f4ceb5b5b88");
const EV_INIT: [u8; 8] = hex!("e432f655cb428625");
const EV_SWAP: [u8; 8] = hex!("1b3c15d58aaabb93");
const EV_SWAP2: [u8; 8] = hex!("bd4233a826507599");

enum P { Init { creator: String, base: String, quote: String, pool: String, bv: String, qv: String }, Trade { pool: String, base: String, quote: String, payer: String } }

pub fn decode(ctx: &Ctx) -> Vec<Event> {
    pair(ctx, |ix: &Ix, _| {
        let d = disc(ix)?; let a = &ix.accounts;
        match d {
            INIT_SPL | INIT_T22 if a.len() > 7 => Some(P::Init { creator: a[2].clone(), base: a[3].clone(), quote: a[4].clone(), pool: a[5].clone(), bv: a[6].clone(), qv: a[7].clone() }),
            SWAP | SWAP2 if a.len() > 9 => Some(P::Trade { pool: a[2].clone(), base: a[7].clone(), quote: a[8].clone(), payer: a[9].clone() }),
            _ => None,
        }
    }, |p, ev: &Ix, ix_index| {
        let ed = event_disc(ev)?; let mut r = Reader::new(event_payload(ev));
        match (p, ed) {
            (P::Init { creator, base, quote, pool, bv, qv }, EV_INIT) => {
                if !ctx.is_stock(quote) { return None; }
                Some(Event::PoolCreated { meta: ctx.meta(ix_index), pool: pool.clone(), base_mint: base.clone(), quote_mint: quote.clone(), creator: creator.clone(), base_vault: Some(bv.clone()), quote_vault: Some(qv.clone()), token: None, holder_rewards: false })
            }
            (P::Trade { pool, base, quote, payer }, EV_SWAP) => {
                if !ctx.is_stock(quote) { return None; }
                let _pool = r.pubkey().ok()?; let _config = r.pubkey().ok()?; let dir = r.u8().ok()?; let _ref = r.bool().ok()?;
                let _amount_in_param = r.u64().ok()?; let _min_out = r.u64().ok()?;
                let actual_in = r.u64().ok()?; let output = r.u64().ok()?; let next_sqrt = r.u128().ok()?;
                Some(swap(ctx, ix_index, pool, base, quote, payer, dir, actual_in, output, next_sqrt))
            }
            (P::Trade { pool, base, quote, payer }, EV_SWAP2) => {
                if !ctx.is_stock(quote) { return None; }
                let _pool = r.pubkey().ok()?; let _config = r.pubkey().ok()?; let dir = r.u8().ok()?; let _ref = r.bool().ok()?;
                let _a0 = r.u64().ok()?; let _a1 = r.u64().ok()?; let _swap_mode = r.u8().ok()?;
                let _incl_in = r.u64().ok()?; let excl_in = r.u64().ok()?; let _left = r.u64().ok()?; let output = r.u64().ok()?; let next_sqrt = r.u128().ok()?;
                Some(swap(ctx, ix_index, pool, base, quote, payer, dir, excl_in, output, next_sqrt))
            }
            _ => None,
        }
    })
}

/// DBC TradeDirection: 0 = BaseToQuote (sell), 1 = QuoteToBase (buy). sqrt_price is sqrt(quote/base) Q64.
fn swap(ctx: &Ctx, ix_index: u16, pool: &str, base: &str, quote: &str, payer: &str, dir: u8, amount_in: u64, amount_out: u64, next_sqrt: u128) -> Event {
    let (side, base_raw, quote_raw) = if dir == 0 { (Side::Sell, amount_in, amount_out) } else { (Side::Buy, amount_out, amount_in) };
    Event::Swap { meta: ctx.meta(ix_index), pool: pool.into(), base_mint: base.into(), quote_mint: quote.into(), wallet: payer.into(), side,
        base_raw: base_raw as u128, quote_raw: quote_raw as u128, reserve_base_raw: None, reserve_quote_raw: None, sqrt_price_q64: Some(next_sqrt), progress_pct: None }
}
