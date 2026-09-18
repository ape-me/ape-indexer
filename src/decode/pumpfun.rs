use super::{disc, event_disc, event_payload, pair, Ctx};
use crate::borsh::Reader;
use crate::events::{Event, Side, TokenMeta};
use crate::tx::Ix;

const CREATE: [u8; 8] = hex!("181ec828051c0777");
const CREATE_V2: [u8; 8] = hex!("d6904cec5f8b31b4");
const BUY: [u8; 8] = hex!("66063d1201daebea");
const BUY_V2: [u8; 8] = hex!("b817ee6167c5d33d");
const BUY_EXACT_QUOTE_IN_V2: [u8; 8] = hex!("c2ab1c46684d5b2f");
const SELL: [u8; 8] = hex!("33e685a4017f83ad");
const SELL_V2: [u8; 8] = hex!("5df6823ce7e940b2");
const EV_CREATE: [u8; 8] = hex!("1b72a94ddeeb6376");
const EV_TRADE: [u8; 8] = hex!("bddb7fd34ee661ee");

enum P { Create, Trade { bonding_curve: String } }

pub fn decode(ctx: &Ctx) -> Vec<Event> {
    pair(ctx, |ix: &Ix, _| {
        let d = disc(ix)?; let a = &ix.accounts;
        match d {
            CREATE | CREATE_V2 => Some(P::Create),
            BUY | SELL if a.len() > 3 => Some(P::Trade { bonding_curve: a[3].clone() }),
            BUY_V2 | BUY_EXACT_QUOTE_IN_V2 | SELL_V2 if a.len() > 10 => Some(P::Trade { bonding_curve: a[10].clone() }),
            _ => None,
        }
    }, |p, ev: &Ix, ix_index| {
        let ed = event_disc(ev)?; let mut r = Reader::new(event_payload(ev));
        match (p, ed) {
            (P::Create, EV_CREATE) => {
                let name = r.string().ok()?; let symbol = r.string().ok()?; let uri = r.string().ok()?;
                let mint = r.pubkey().ok()?; let bonding_curve = r.pubkey().ok()?; let _user = r.pubkey().ok()?; let creator = r.pubkey().ok()?;
                let _ts = r.i64().ok()?; r.skip(4 * 8).ok()?; // virtual/real reserves, supply
                let _token_program = r.pubkey().ok()?; let _mayhem = r.bool().ok()?; let _cashback = r.bool().ok()?;
                let quote_mint = r.pubkey().ok()?; let _vq = r.u64().ok()?; let _cfb = r.u64().ok()?; let holder_rewards = r.bool().unwrap_or(false);
                if !ctx.is_stock(&quote_mint) { return None; }
                Some(Event::PoolCreated { meta: ctx.meta(ix_index), pool: bonding_curve, base_mint: mint, quote_mint, creator, base_vault: None, quote_vault: None,
                    token: Some(TokenMeta { name, symbol, uri, decimals: Some(6) }), holder_rewards })
            }
            (P::Trade { bonding_curve }, EV_TRADE) => {
                let mint = r.pubkey().ok()?; let _sol_amount = r.u64().ok()?; let token_amount = r.u64().ok()?; let is_buy = r.bool().ok()?;
                let user = r.pubkey().ok()?; let _ts = r.i64().ok()?;
                let _vsol = r.u64().ok()?; let virtual_token = r.u64().ok()?; let _rsol = r.u64().ok()?; let real_token = r.u64().ok()?;
                let _fee_recipient = r.pubkey().ok()?; let _fbps = r.u64().ok()?; let _fee = r.u64().ok()?;
                let _creator = r.pubkey().ok()?; let _cfbps = r.u64().ok()?; let _cfee = r.u64().ok()?;
                let _track = r.bool().ok()?; r.skip(3 * 8).ok()?; let _lu = r.i64().ok()?; let _ix_name = r.string().ok()?;
                let _mayhem = r.bool().ok()?; r.skip(4 * 8).ok()?; // cashback bps, cashback, buyback bps, buyback
                let n_share = r.u32().ok()? as usize;
                for _ in 0..n_share { let _ = r.pubkey().ok()?; let _ = r.u64().ok()?; } // Shareholder { address, bps }
                let quote_mint = r.pubkey().ok()?; let quote_amount = r.u64().ok()?; let virtual_quote = r.u64().ok()?; let _real_quote = r.u64().ok()?;
                if !ctx.is_stock(&quote_mint) { return None; }
                Some(Event::Swap { meta: ctx.meta(ix_index), pool: bonding_curve.clone(), base_mint: mint, quote_mint, wallet: user, side: if is_buy { Side::Buy } else { Side::Sell },
                    base_raw: token_amount as u128, quote_raw: quote_amount as u128,
                    reserve_base_raw: Some(virtual_token as u128), reserve_quote_raw: Some(virtual_quote as u128), sqrt_price_q64: None,
                    progress_pct: Some(((1.0 - real_token as f64 / 793_100_000_000_000.0) * 100.0).clamp(0.0, 100.0)) })
            }
            _ => None,
        }
    })
}
