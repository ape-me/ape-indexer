//! The two primitives every program is reduced to.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Program { Launchlab, Cpmm, Pumpfun, Pumpswap, Dbc, Damm2 }

impl Program {
    pub const ALL: [Program; 6] = [Program::Launchlab, Program::Cpmm, Program::Pumpfun, Program::Pumpswap, Program::Dbc, Program::Damm2];
    pub fn id(self) -> &'static str {
        match self {
            Program::Launchlab => "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj",
            Program::Cpmm => "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C",
            Program::Pumpfun => "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
            Program::Pumpswap => "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA",
            Program::Dbc => "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN",
            Program::Damm2 => "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG",
        }
    }
    pub fn from_id(id: &str) -> Option<Program> { Program::ALL.into_iter().find(|p| p.id() == id) }
    pub fn name(self) -> &'static str {
        match self { Program::Launchlab => "launchlab", Program::Cpmm => "cpmm", Program::Pumpfun => "pumpfun", Program::Pumpswap => "pumpswap", Program::Dbc => "dbc", Program::Damm2 => "damm2" }
    }
    /// curve programs create tokens; amm programs only graduate tokens we already know
    pub fn is_curve(self) -> bool { matches!(self, Program::Launchlab | Program::Pumpfun | Program::Dbc) }
    pub fn launchpad(self) -> &'static str {
        match self { Program::Launchlab => "stonkfun", Program::Pumpfun => "pumpfun", Program::Dbc => "dbc", _ => "" }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side { Buy, Sell }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta { pub program: Program, pub signature: String, pub slot: u64, pub block_time: i64, pub ix_index: u16 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMeta { pub name: String, pub symbol: String, pub uri: String, pub decimals: Option<u8> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    PoolCreated {
        #[serde(flatten)] meta: Meta,
        pool: String, base_mint: String, quote_mint: String, creator: String,
        base_vault: Option<String>, quote_vault: Option<String>,
        /// name/symbol/uri when the program puts them in the event (LaunchLab, pump.fun)
        token: Option<TokenMeta>,
        holder_rewards: bool,
    },
    Swap {
        #[serde(flatten)] meta: Meta,
        pool: String, base_mint: String, quote_mint: String, wallet: String, side: Side,
        /// amounts in raw units of each mint
        base_raw: u128, quote_raw: u128,
        /// post-trade pool reserves in raw units when the program reports them. price = quote/base
        reserve_base_raw: Option<u128>, reserve_quote_raw: Option<u128>,
        /// for DBC-style pools: sqrt price Q64.64, quote per base in raw units
        sqrt_price_q64: Option<u128>,
    },
}

impl Event {
    pub fn meta(&self) -> &Meta { match self { Event::PoolCreated { meta, .. } | Event::Swap { meta, .. } => meta } }
    /// (program, kind) labels for metrics.
    pub fn labels(&self) -> (&'static str, &'static str) {
        let p = match self.meta().program { Program::Launchlab => "launchlab", Program::Cpmm => "cpmm", Program::Pumpfun => "pumpfun", Program::Pumpswap => "pumpswap", Program::Dbc => "dbc", Program::Damm2 => "damm2" };
        (p, match self { Event::PoolCreated { .. } => "pool", Event::Swap { .. } => "swap" })
    }
    /// price in raw quote units per raw base unit. Caller scales by 10^(base_dec - quote_dec).
    pub fn raw_price(&self) -> Option<f64> {
        match self {
            Event::Swap { reserve_base_raw: Some(b), reserve_quote_raw: Some(q), .. } if *b > 0 => Some(*q as f64 / *b as f64),
            Event::Swap { sqrt_price_q64: Some(s), .. } => { let f = *s as f64 / 18446744073709551616.0; Some(f * f) }
            Event::Swap { base_raw, quote_raw, .. } if *base_raw > 0 => Some(*quote_raw as f64 / *base_raw as f64),
            _ => None,
        }
    }
}
