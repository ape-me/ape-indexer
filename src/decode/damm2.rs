use super::{disc, event_disc, event_payload, orient, pair, Ctx};
use crate::borsh::Reader;
use crate::events::{Event, Side};
use crate::tx::Ix;

const INIT_POOL: [u8; 8] = hex!("5fb40aac54aee828");
const INIT_POOL_DYN: [u8; 8] = hex!("955248c5fdfc440f");
const SWAP: [u8; 8] = hex!("f8c69e91e17587c8");
const SWAP2: [u8; 8] = hex!("414b3f4ceb5b5b88");
const EV_INIT: [u8; 8] = hex!("e432f655cb428625");
const EV_SWAP2: [u8; 8] = hex!("bd4233a826507599");

enum P { Init { creator: String, pool: String, a: String, b: String, av: String, bv: String }, Trade { pool: String, a: String, b: String, payer: String } }

pub fn decode(ctx: &Ctx) -> Vec<Event> {
    pair(ctx, |ix: &Ix, _| {
        let d = disc(ix)?; let acc = &ix.accounts;
        match d {
            INIT_POOL if acc.len() > 11 => Some(P::Init { creator: acc[0].clone(), pool: acc[6].clone(), a: acc[8].clone(), b: acc[9].clone(), av: acc[10].clone(), bv: acc[11].clone() }),
            INIT_POOL_DYN if acc.len() > 12 => Some(P::Init { creator: acc[0].clone(), pool: acc[7].clone(), a: acc[9].clone(), b: acc[10].clone(), av: acc[11].clone(), bv: acc[12].clone() }),
            SWAP | SWAP2 if acc.len() > 8 => Some(P::Trade { pool: acc[1].clone(), a: acc[6].clone(), b: acc[7].clone(), payer: acc[8].clone() }),
            _ => None,
        }
    }, |p, ev: &Ix, ix_index| {
        let ed = event_disc(ev)?; let mut r = Reader::new(event_payload(ev));
        match (p, ed) {
            (P::Init { creator, pool, a, b, av, bv }, EV_INIT) => {
                let (base, quote) = orient(a, b, ctx.stocks)?;
                let (bvault, qvault) = if &base == a { (av.clone(), bv.clone()) } else { (bv.clone(), av.clone()) };
                Some(Event::PoolCreated { meta: ctx.meta(ix_index), pool: pool.clone(), base_mint: base, quote_mint: quote, creator: creator.clone(), base_vault: Some(bvault), quote_vault: Some(qvault), token: None, holder_rewards: false })
            }
            (P::Trade { pool, a, b, payer }, EV_SWAP2) => {
                let (base, quote) = orient(a, b, ctx.stocks)?;
                let _pool = r.pubkey().ok()?; let dir = r.u8().ok()?; let _cfm = r.u8().ok()?; let _ref = r.bool().ok()?;
                let _a0 = r.u64().ok()?; let _a1 = r.u64().ok()?; let _mode = r.u8().ok()?;
                let _incl_in = r.u64().ok()?; let excl_in = r.u64().ok()?; let _left = r.u64().ok()?; let output = r.u64().ok()?; let _sqrt = r.u128().ok()?;
                r.skip(4 * 8).ok()?; // claiming_fee, protocol_fee, compounding_fee, referral_fee
                let _tf_in = r.u64().ok()?; let _tf_out = r.u64().ok()?; let _ex_out = r.u64().ok()?; let _ts = r.u64().ok()?;
                let reserve_a = r.u64().ok()?; let reserve_b = r.u64().ok()?;
                // dir 0 = AtoB, 1 = BtoA. a_is_base tells which side is the meme.
                let a_is_base = &base == a;
                let quote_in = (dir == 0) != a_is_base; // AtoB with A=quote → quote in; BtoA with B=quote → quote in
                let (side, base_raw, quote_raw) = if quote_in { (Side::Buy, output, excl_in) } else { (Side::Sell, excl_in, output) };
                let (rb, rq) = if a_is_base { (reserve_a, reserve_b) } else { (reserve_b, reserve_a) };
                Some(Event::Swap { meta: ctx.meta(ix_index), pool: pool.clone(), base_mint: base, quote_mint: quote, wallet: payer.clone(), side,
                    base_raw: base_raw as u128, quote_raw: quote_raw as u128, reserve_base_raw: Some(rb as u128), reserve_quote_raw: Some(rq as u128), sqrt_price_q64: None, progress_pct: None })
            }
            _ => None,
        }
    })
}
