//! Reduce a transaction to PoolCreated / Swap events across six programs.
//! Rule: an Anchor CPI event is an inner instruction of the same program whose data starts with
//! EVENT_TAG then the 8-byte event discriminator. We walk instructions in execution order and pair
//! each event with the most recent unmatched swap/init instruction of that program.
#[macro_export]
macro_rules! hex { ($s:literal) => { $crate::decode::hex8($s) } }
pub const fn hex8(s: &str) -> [u8; 8] {
    let b = s.as_bytes(); let mut out = [0u8; 8]; let mut i = 0;
    while i < 8 { out[i] = (hexval(b[2 * i]) << 4) | hexval(b[2 * i + 1]); i += 1; }
    out
}
const fn hexval(c: u8) -> u8 { match c { b'0'..=b'9' => c - b'0', b'a'..=b'f' => c - b'a' + 10, b'A'..=b'F' => c - b'A' + 10, _ => 0 } }

pub mod launchlab; pub mod cpmm; pub mod pumpfun; pub mod pumpswap; pub mod dbc; pub mod damm2;

use crate::events::{Event, Meta, Program};
use crate::tx::{Ix, TxView};
use std::collections::HashSet;

pub const EVENT_TAG: [u8; 8] = [0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d];

pub fn disc(ix: &Ix) -> Option<[u8; 8]> { ix.data.get(..8)?.try_into().ok() }
pub fn event_disc(ix: &Ix) -> Option<[u8; 8]> {
    if ix.data.len() < 16 || ix.data[..8] != EVENT_TAG { return None; }
    ix.data[8..16].try_into().ok()
}
pub fn event_payload(ix: &Ix) -> &[u8] { &ix.data[16..] }

/// Everything a program decoder needs to see.
pub struct Ctx<'a> { pub tx: &'a TxView, pub stocks: &'a HashSet<String>, pub program: Program }

impl<'a> Ctx<'a> {
    pub fn meta(&self, ix_index: u16) -> Meta {
        Meta { program: self.program, signature: self.tx.signature.clone(), slot: self.tx.slot, block_time: self.tx.block_time, ix_index }
    }
    pub fn is_stock(&self, mint: &str) -> bool { self.stocks.contains(mint) }
}

/// Given two mints, return (base, quote) if exactly one is a stock.
pub fn orient(a: &str, b: &str, stocks: &HashSet<String>) -> Option<(String, String)> {
    match (stocks.contains(a), stocks.contains(b)) {
        (false, true) => Some((a.to_string(), b.to_string())),
        (true, false) => Some((b.to_string(), a.to_string())),
        _ => None,
    }
}

pub fn decode(tx: &TxView, stocks: &HashSet<String>) -> Vec<Event> {
    let mut out = Vec::new();
    if tx.failed { return out; }
    for p in Program::ALL {
        if !tx.touches(p.id()) { continue; }
        let ctx = Ctx { tx, stocks, program: p };
        let evs = match p {
            Program::Launchlab => launchlab::decode(&ctx),
            Program::Cpmm => cpmm::decode(&ctx),
            Program::Pumpfun => pumpfun::decode(&ctx),
            Program::Pumpswap => pumpswap::decode(&ctx),
            Program::Dbc => dbc::decode(&ctx),
            Program::Damm2 => damm2::decode(&ctx),
        };
        out.extend(evs);
    }
    out
}

/// Pair helper: walk ixs of `program`; `on_ix` returns Some(pending) for an instruction of interest,
/// `on_event` consumes (pending, event ix) and may emit events. Pending items are matched LIFO.
pub fn pair<P, E>(ctx: &Ctx, mut on_ix: impl FnMut(&Ix, u16) -> Option<P>, mut on_event: impl FnMut(&P, &Ix, u16) -> Option<E>) -> Vec<E> {
    let pid = ctx.program.id();
    let mut pending: Vec<P> = Vec::new();
    let mut out = Vec::new();
    for (i, ix) in ctx.tx.ixs.iter().enumerate() {
        if ix.program != pid { continue; }
        if event_disc(ix).is_some() {
            if let Some(p) = pending.pop() { if let Some(e) = on_event(&p, ix, i as u16) { out.push(e); } }
        } else if let Some(p) = on_ix(ix, i as u16) { pending.push(p); }
    }
    out
}
