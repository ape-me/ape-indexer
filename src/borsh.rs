//! Minimal borsh reader for Anchor event payloads. No allocation beyond strings.
use anyhow::{bail, Result};

pub struct Reader<'a> { buf: &'a [u8], pos: usize }

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self { Self { buf, pos: 0 } }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.buf.len() { bail!("borsh: short read at {} need {}", self.pos, n); }
        let s = &self.buf[self.pos..self.pos + n]; self.pos += n; Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8> { Ok(self.take(1)?[0]) }
    pub fn bool(&mut self) -> Result<bool> { Ok(self.u8()? != 0) }
    pub fn pos(&self) -> usize { self.pos }
    pub fn u32(&mut self) -> Result<u32> { Ok(u32::from_le_bytes(self.take(4)?.try_into()?)) }
    pub fn u16(&mut self) -> Result<u16> { Ok(u16::from_le_bytes(self.take(2)?.try_into()?)) }
    pub fn u64(&mut self) -> Result<u64> { Ok(u64::from_le_bytes(self.take(8)?.try_into()?)) }
    pub fn i64(&mut self) -> Result<i64> { Ok(i64::from_le_bytes(self.take(8)?.try_into()?)) }
    pub fn u128(&mut self) -> Result<u128> { Ok(u128::from_le_bytes(self.take(16)?.try_into()?)) }
    pub fn i128(&mut self) -> Result<i128> { Ok(i128::from_le_bytes(self.take(16)?.try_into()?)) }
    pub fn pubkey(&mut self) -> Result<String> { Ok(bs58::encode(self.take(32)?).into_string()) }
    pub fn string(&mut self) -> Result<String> {
        let n = u32::from_le_bytes(self.take(4)?.try_into()?) as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    pub fn skip(&mut self, n: usize) -> Result<()> { self.take(n).map(|_| ()) }
}
