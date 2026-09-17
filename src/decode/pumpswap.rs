use super::{disc, event_disc, event_payload, orient, pair, Ctx};
use crate::borsh::Reader;
use crate::events::{Event, Side};
use crate::tx::Ix;

const CREATE_POOL: [u8; 8] = hex!("e992d18ecf6840bc");
const BUY: [u8; 8] = hex!("66063d1201daebea");
const BUY_EXACT_QUOTE_IN: [u8; 8] = hex!("c62e1552b4d9e870");
const SELL: [u8; 8] = hex!("33e685a4017f83ad");
const EV_CREATE_POOL: [u8; 8] = hex!("b1310cd2a076a774");
const EV_BUY: [u8; 8] = hex!("67f4521f2cf57777");
const EV_SELL: [u8; 8] = hex!("3e2f370aa503dc2a");

enum P { Create { bv: String, qv: String }, Trade { base: String, quote: String } }

pub fn decode(ctx: &Ctx) -> Vec<Event> {
    pair(ctx, |ix: &Ix, _| {
        let d = disc(ix)?; let a = &ix.accounts;
        match d {
            CREATE_POOL if a.len() > 10 => Some(P::Create { bv: a[9].clone(), qv: a[10].clone() }),
            BUY | BUY_EXACT_QUOTE_IN | SELL if a.len() > 4 => Some(P::Trade { base: a[3].clone(), quote: a[4].clone() }),
            _ => None,
        }
    }, |p, ev: &Ix, ix_index| {
        let ed = event_disc(ev)?; let mut r = Reader::new(event_payload(ev));
        match (p, ed) {
            (P::Create { bv, qv }, EV_CREATE_POOL) => {
                let _ts = r.i64().ok()?; let _index = r.u16().ok()?; let creator = r.pubkey().ok()?;
                let base_mint = r.pubkey().ok()?; let quote_mint = r.pubkey().ok()?;
                r.skip(2 + 7 * 8 + 1).ok()?; // decimals x2, amounts x7, bump
                let pool = r.pubkey().ok()?;
                let _lp = r.pubkey().ok()?; let _uba = r.pubkey().ok()?; let _uqa = r.pubkey().ok()?; let _cc = r.pubkey().ok()?;
                let _mayhem = r.bool().ok()?; let _cfb = r.u64().ok()?; let _edit = r.bool().ok()?; let holder_rewards = r.bool().unwrap_or(false);
                let (b, q) = orient(&base_mint, &quote_mint, ctx.stocks)?;
                if q != quote_mint { return None; } // stock must be the quote side on PumpSwap
                Some(Event::PoolCreated { meta: ctx.meta(ix_index), pool, base_mint: b, quote_mint: q, creator, base_vault: Some(bv.clone()), quote_vault: Some(qv.clone()), token: None, holder_rewards })
            }
            (P::Trade { base, quote }, EV_BUY) => {
                if !ctx.is_stock(quote) { return None; }
                let _ts = r.i64().ok()?; let base_out = r.u64().ok()?; let _max_q = r.u64().ok()?; r.skip(2 * 8).ok()?;
                let pool_base = r.u64().ok()?; let pool_quote = r.u64().ok()?; let quote_in = r.u64().ok()?;
                r.skip(6 * 8).ok()?; // lp fee bps, lp fee, proto bps, proto fee, q_with_lp, user_q_in
                let pool = r.pubkey().ok()?; let user = r.pubkey().ok()?;
                for _ in 0..5 { r.pubkey().ok()?; } // user accounts, fee recipients, coin_creator
                r.skip(2 * 8).ok()?; let _track = r.bool().ok()?; r.skip(3 * 8).ok()?; let _lu = r.i64().ok()?; let _min_out = r.u64().ok()?; let _ix_name = r.string().ok()?;
                r.skip(4 * 8).ok()?; // cashback bps, cashback, buyback bps, buyback
                let virtual_quote = r.i128().unwrap_or(0).max(0) as u128;
                // event reserves are pre-trade; custom pairs add virtual quote reserves to the curve
                let rb = pool_base as u128 - base_out as u128; let rq = pool_quote as u128 + virtual_quote + quote_in as u128;
                Some(Event::Swap { meta: ctx.meta(ix_index), pool, base_mint: base.clone(), quote_mint: quote.clone(), wallet: user, side: Side::Buy,
                    base_raw: base_out as u128, quote_raw: quote_in as u128, reserve_base_raw: Some(rb), reserve_quote_raw: Some(rq), sqrt_price_q64: None })
            }
            (P::Trade { base, quote }, EV_SELL) => {
                if !ctx.is_stock(quote) { return None; }
                let _ts = r.i64().ok()?; let base_in = r.u64().ok()?; let _min_q = r.u64().ok()?; r.skip(2 * 8).ok()?;
                let pool_base = r.u64().ok()?; let pool_quote = r.u64().ok()?; let quote_out = r.u64().ok()?;
                r.skip(6 * 8).ok()?;
                let pool = r.pubkey().ok()?; let user = r.pubkey().ok()?;
                for _ in 0..5 { r.pubkey().ok()?; }
                r.skip(6 * 8).ok()?; // creator bps, creator fee, cashback bps, cashback, buyback bps, buyback
                let virtual_quote = r.i128().unwrap_or(0).max(0) as u128;
                let rb = pool_base as u128 + base_in as u128; let rq = (pool_quote as u128 + virtual_quote).saturating_sub(quote_out as u128);
                Some(Event::Swap { meta: ctx.meta(ix_index), pool, base_mint: base.clone(), quote_mint: quote.clone(), wallet: user, side: Side::Sell,
                    base_raw: base_in as u128, quote_raw: quote_out as u128, reserve_base_raw: Some(rb), reserve_quote_raw: Some(rq), sqrt_price_q64: None })
            }
            _ => None,
        }
    })
}
