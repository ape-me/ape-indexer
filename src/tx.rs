//! One transaction shape for both sources: RPC JSON (tests, backfill) and Yellowstone proto (stream).
use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Ix {
    pub program: String,
    pub accounts: Vec<String>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TxView {
    pub signature: String,
    pub slot: u64,
    pub block_time: i64,
    pub failed: bool,
    pub keys: Vec<String>,
    /// all instructions in execution order: outer i, then its inner ones, then outer i+1 ...
    pub ixs: Vec<Ix>,
    pub logs: Vec<String>,
}

impl TxView {
    pub fn touches(&self, key: &str) -> bool { self.keys.iter().any(|k| k == key) }

    /// RPC getTransaction with encoding "json" and maxSupportedTransactionVersion 0
    pub fn from_rpc_json(v: &Value) -> Result<TxView> {
        let r = v.get("result").unwrap_or(v);
        let tx = &r["transaction"]; let meta = &r["meta"];
        let mut keys: Vec<String> = tx["message"]["accountKeys"].as_array().context("accountKeys")?.iter().map(|k| k.as_str().unwrap_or("").to_string()).collect();
        for side in ["writable", "readonly"] {
            if let Some(a) = meta["loadedAddresses"][side].as_array() { keys.extend(a.iter().map(|k| k.as_str().unwrap_or("").to_string())); }
        }
        let mk = |ix: &Value| -> Ix {
            let pi = ix["programIdIndex"].as_u64().unwrap_or(0) as usize;
            Ix {
                program: keys.get(pi).cloned().unwrap_or_default(),
                accounts: ix["accounts"].as_array().map(|a| a.iter().map(|i| keys.get(i.as_u64().unwrap_or(0) as usize).cloned().unwrap_or_default()).collect()).unwrap_or_default(),
                data: bs58::decode(ix["data"].as_str().unwrap_or("")).into_vec().unwrap_or_default(),
            }
        };
        let outer: Vec<&Value> = tx["message"]["instructions"].as_array().context("instructions")?.iter().collect();
        let inner = meta["innerInstructions"].as_array().cloned().unwrap_or_default();
        let mut ixs = Vec::new();
        for (i, ix) in outer.iter().enumerate() {
            ixs.push(mk(ix));
            for grp in &inner {
                if grp["index"].as_u64() == Some(i as u64) {
                    for iix in grp["instructions"].as_array().unwrap_or(&vec![]) { ixs.push(mk(iix)); }
                }
            }
        }
        Ok(TxView {
            signature: tx["signatures"][0].as_str().unwrap_or("").to_string(),
            slot: r["slot"].as_u64().unwrap_or(0),
            block_time: r["blockTime"].as_i64().unwrap_or(0),
            failed: !meta["err"].is_null(),
            keys,
            ixs,
            logs: meta["logMessages"].as_array().map(|a| a.iter().filter_map(|l| l.as_str().map(String::from)).collect()).unwrap_or_default(),
        })
    }

    /// Yellowstone SubscribeUpdateTransaction
    pub fn from_geyser(u: &yellowstone_grpc_proto::geyser::SubscribeUpdateTransaction, block_time: i64) -> Result<TxView> {
        let info = u.transaction.as_ref().context("no transaction info")?;
        let tx = info.transaction.as_ref().context("no transaction")?;
        let msg = tx.message.as_ref().context("no message")?;
        let meta = info.meta.as_ref().context("no meta")?;
        let mut keys: Vec<String> = msg.account_keys.iter().map(|k| bs58::encode(k).into_string()).collect();
        keys.extend(meta.loaded_writable_addresses.iter().map(|k| bs58::encode(k).into_string()));
        keys.extend(meta.loaded_readonly_addresses.iter().map(|k| bs58::encode(k).into_string()));
        let key = |i: usize| keys.get(i).cloned().unwrap_or_default();
        let mut ixs = Vec::new();
        for (i, ix) in msg.instructions.iter().enumerate() {
            ixs.push(Ix { program: key(ix.program_id_index as usize), accounts: ix.accounts.iter().map(|a| key(*a as usize)).collect(), data: ix.data.clone() });
            for grp in meta.inner_instructions.iter().filter(|g| g.index as usize == i) {
                for iix in &grp.instructions {
                    ixs.push(Ix { program: key(iix.program_id_index as usize), accounts: iix.accounts.iter().map(|a| key(*a as usize)).collect(), data: iix.data.clone() });
                }
            }
        }
        Ok(TxView {
            signature: bs58::encode(&info.signature).into_string(),
            slot: u.slot,
            block_time,
            failed: meta.err.is_some(),
            keys,
            ixs,
            logs: meta.log_messages.clone(),
        })
    }
}
