use super::{disc, event_disc, event_payload, pair, Ctx};
use crate::borsh::Reader;
use crate::events::{Event, Side, TokenMeta};
use crate::tx::Ix;

const INITIALIZE: [u8; 8] = hex!("afaf6d1f0d989bed");
const INITIALIZE_V2: [u8; 8] = hex!("4399af27da102620");
const BUY_EXACT_IN: [u8; 8] = hex!("faea0d7bd59c13ec");
const BUY_EXACT_OUT: [u8; 8] = hex!("18d3742869039938");
const SELL_EXACT_IN: [u8; 8] = hex!("9527de9bd37c981a");
const SELL_EXACT_OUT: [u8; 8] = hex!("5fc8472208090ba6");
const EV_POOL_CREATE: [u8; 8] = hex!("97d7e20976a173ae");
const EV_TRADE: [u8; 8] = hex!("bddb7fd34ee661ee");

enum P { Init { pool: String, creator: String, base: String, quote: String, bv: String, qv: String }, Trade { payer: String, pool: String, base: String, quote: String } }

pub fn decode(ctx: &Ctx) -> Vec<Event> {
    pair(ctx, |ix: &Ix, _| {
        let d = disc(ix)?; let a = &ix.accounts;
        match d {
            INITIALIZE | INITIALIZE_V2 if a.len() > 9 => Some(P::Init { pool: a[5].clone(), creator: a[1].clone(), base: a[6].clone(), quote: a[7].clone(), bv: a[8].clone(), qv: a[9].clone() }),
            BUY_EXACT_IN | BUY_EXACT_OUT | SELL_EXACT_IN | SELL_EXACT_OUT if a.len() > 10 => Some(P::Trade { payer: a[0].clone(), pool: a[4].clone(), base: a[9].clone(), quote: a[10].clone() }),
            _ => None,
        }
    }, |p, ev: &Ix, ix_index| {
        let ed = event_disc(ev)?; let mut r = Reader::new(event_payload(ev));
        match (p, ed) {
            (P::Init { pool, creator, base, quote, bv, qv }, EV_POOL_CREATE) => {
                if !ctx.is_stock(quote) { return None; }
                let _pool = r.pubkey().ok()?; let _creator = r.pubkey().ok()?; let _config = r.pubkey().ok()?;
                let decimals = r.u8().ok()?; let name = r.string().ok()?; let symbol = r.string().ok()?; let uri = r.string().ok()?;
                Some(Event::PoolCreated { meta: ctx.meta(ix_index), pool: pool.clone(), base_mint: base.clone(), quote_mint: quote.clone(), creator: creator.clone(),
                    base_vault: Some(bv.clone()), quote_vault: Some(qv.clone()), token: Some(TokenMeta { name, symbol, uri, decimals: Some(decimals) }), holder_rewards: false })
            }
            (P::Trade { payer, pool, base, quote }, EV_TRADE) => {
                if !ctx.is_stock(quote) { return None; }
                let _pool = r.pubkey().ok()?; let _total_base_sell = r.u64().ok()?;
                let virtual_base = r.u64().ok()?; let virtual_quote = r.u64().ok()?;
                let _rb0 = r.u64().ok()?; let _rq0 = r.u64().ok()?;
                let real_base_after = r.u64().ok()?; let real_quote_after = r.u64().ok()?;
                let amount_in = r.u64().ok()?; let amount_out = r.u64().ok()?;
                r.skip(4 * 8).ok()?; // protocol, platform, creator, share fees
                let dir = r.u8().ok()?; // 0 buy (quote in, base out), 1 sell
                let (side, base_raw, quote_raw) = if dir == 0 { (Side::Buy, amount_out, amount_in) } else { (Side::Sell, amount_in, amount_out) };
                Some(Event::Swap { meta: ctx.meta(ix_index), pool: pool.clone(), base_mint: base.clone(), quote_mint: quote.clone(), wallet: payer.clone(), side,
                    base_raw: base_raw as u128, quote_raw: quote_raw as u128,
                    reserve_base_raw: Some((virtual_base - real_base_after) as u128), reserve_quote_raw: Some((virtual_quote + real_quote_after) as u128), sqrt_price_q64: None })
            }
            _ => None,
        }
    })
}
