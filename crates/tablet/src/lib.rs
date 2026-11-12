//! 固定长度的转账简牍。
//!
//! 布局（大端，共 [`TABLET_LEN`] 字节，多一字节或少一字节都拒绝）：
//!
//! ```text
//!  0  4  魔数 KHUB
//!  4  1  版本 = 1
//!  5  1  家族（目前只有 1 = EVM）
//!  6  8  链编号
//! 14  1  资产种类（1 主币，2 ERC-20 transfer）
//! 15 20  代币合约（主币必须全零）
//! 35 20  收款地址
//! 55 32  数量
//! 87 32  费用上限
//! 119 8  过期 unix 秒
//! 127 8  nonce
//! 135 8  gas_limit
//! 143 32  max_fee_per_gas
//! 175 32  max_priority_fee_per_gas
//! ```
//!
//! 没有名称、没有自由调用数据、没有授权。调用数据由资产种类唯一决定。

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::fmt;

pub const MAGIC: &[u8; 4] = b"KHUB";
pub const RECEIPT_MAGIC: &[u8; 4] = b"KHUR";
pub const VERSION: u8 = 1;
pub const FAMILY_EVM: u8 = 1;
pub const ASSET_NATIVE: u8 = 1;
pub const ASSET_ERC20: u8 = 2;

pub const TABLET_LEN: usize = 207;
pub const MAX_RAW_TX: usize = 512;
pub const MAX_GAS: u64 = 200_000;
/// 简牍从写出到签名，最多活这么久。过期的和过远的都拒绝。
pub const MAX_TTL_SECS: u64 = 30 * 60;

const OFF_FAMILY: usize = 5;
const OFF_CHAIN: usize = 6;
const OFF_ASSET: usize = 14;
const OFF_TOKEN: usize = 15;
const OFF_TO: usize = 35;
const OFF_AMOUNT: usize = 55;
const OFF_FEE_CAP: usize = 87;
const OFF_EXPIRY: usize = 119;
const OFF_NONCE: usize = 127;
const OFF_GAS: usize = 135;
const OFF_MAX_FEE: usize = 143;
const OFF_PRIORITY: usize = 175;

const _: () = assert!(OFF_PRIORITY + 32 == TABLET_LEN);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct U256([u8; 32]);

impl U256 {
    pub const fn zero() -> Self {
        Self([0u8; 32])
    }

    pub fn from_u64(v: u64) -> Self {
        let mut out = [0u8; 32];
        out[24..].copy_from_slice(&v.to_be_bytes());
        Self(out)
    }

    pub fn from_be_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn to_be_bytes(self) -> [u8; 32] {
        self.0
    }

    pub fn is_zero(self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }

    pub fn to_u64(self) -> Option<u64> {
        if self.0[..24].iter().any(|b| *b != 0) {
            return None;
        }
        let mut limb = [0u8; 8];
        limb.copy_from_slice(&self.0[24..]);
        Some(u64::from_be_bytes(limb))
    }

    pub fn cmp(self, other: Self) -> Ordering {
        self.0.cmp(&other.0)
    }

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        let mut a = self.limbs();
        let b = rhs.limbs();
        let mut carry = 0u128;
        for i in 0..4 {
            let sum = a[i] as u128 + b[i] as u128 + carry;
            a[i] = sum as u64;
            carry = sum >> 64;
        }
        if carry != 0 {
            None
        } else {
            Some(Self::from_limbs(a))
        }
    }

    pub fn checked_mul_u64(self, rhs: u64) -> Option<Self> {
        let limbs = self.limbs();
        let rhs = rhs as u128;
        let mut out = [0u64; 5];
        let mut carry = 0u128;
        for i in 0..4 {
            let prod = limbs[i] as u128 * rhs + carry;
            out[i] = prod as u64;
            carry = prod >> 64;
        }
        out[4] = carry as u64;
        if out[4] != 0 {
            return None;
        }
        Some(Self::from_limbs([out[0], out[1], out[2], out[3]]))
    }

    pub fn from_dec_str(input: &str) -> Option<Self> {
        if input.is_empty() || !input.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut acc = Self::zero();
        for b in input.bytes() {
            acc = acc.checked_mul_u64(10)?;
            acc = acc.checked_add(Self::from_u64((b - b'0') as u64))?;
        }
        Some(acc)
    }

    pub fn to_dec_string(self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        let mut limbs = self.limbs();
        let mut digits = Vec::new();
        while limbs.iter().any(|l| *l != 0) {
            let mut rem = 0u128;
            for i in (0..4).rev() {
                let cur = (rem << 64) | limbs[i] as u128;
                limbs[i] = (cur / 10) as u64;
                rem = cur % 10;
            }
            digits.push(b'0' + rem as u8);
        }
        digits.reverse();
        String::from_utf8(digits).unwrap_or_else(|_| "0".to_string())
    }

    fn limbs(self) -> [u64; 4] {
        let mut out = [0u64; 4];
        for i in 0..4 {
            let start = 24 - i * 8;
            let mut limb = [0u8; 8];
            limb.copy_from_slice(&self.0[start..start + 8]);
            out[i] = u64::from_be_bytes(limb);
        }
        out
    }

    fn from_limbs(limbs: [u64; 4]) -> Self {
        let mut out = [0u8; 32];
        for i in 0..4 {
            let start = 24 - i * 8;
            out[start..start + 8].copy_from_slice(&limbs[i].to_be_bytes());
        }
        Self(out)
    }
}

impl fmt::Debug for U256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "U256({})", self.to_dec_string())
    }
}

#[derive(Debug)]
pub struct Error(&'static str);

impl Error {
    pub const fn new(msg: &'static str) -> Self {
        Self(msg)
    }

    pub fn msg(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Tablet {
    family: u8,
    chain_id: u64,
    asset_kind: u8,
    token: [u8; 20],
    recipient: [u8; 20],
    amount: U256,
    fee_cap: U256,
    expiry: u64,
    nonce: u64,
    gas_limit: u64,
    max_fee_per_gas: U256,
    max_priority_fee_per_gas: U256,
}

impl fmt::Debug for Tablet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tablet")
            .field("family", &self.family)
            .field("chain_id", &self.chain_id)
            .field("asset_kind", &self.asset_kind)
            .field("amount", &self.amount)
            .field("fee_cap", &self.fee_cap)
            .field("expiry", &self.expiry)
            .field("nonce", &self.nonce)
            .field("gas_limit", &self.gas_limit)
            .finish()
    }
}

impl Tablet {
    pub fn new(
        family: u8,
        chain_id: u64,
        asset_kind: u8,
        token: [u8; 20],
        recipient: [u8; 20],
        amount: U256,
        fee_cap: U256,
        expiry: u64,
        nonce: u64,
        gas_limit: u64,
        max_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    ) -> Result<Self, Error> {
        let tablet = Self {
            family,
            chain_id,
            asset_kind,
            token,
            recipient,
            amount,
            fee_cap,
            expiry,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
        };
        tablet.check_shape()?;
        Ok(tablet)
    }

    pub fn family(self) -> u8 {
        self.family
    }
    pub fn chain_id(self) -> u64 {
        self.chain_id
    }
    pub fn asset_kind(self) -> u8 {
        self.asset_kind
    }
    pub fn token(self) -> [u8; 20] {
        self.token
    }
    pub fn recipient(self) -> [u8; 20] {
        self.recipient
    }
    pub fn amount(self) -> U256 {
        self.amount
    }
    pub fn fee_cap(self) -> U256 {
        self.fee_cap
    }
    pub fn expiry(self) -> u64 {
        self.expiry
    }
    pub fn nonce(self) -> u64 {
        self.nonce
    }
    pub fn gas_limit(self) -> u64 {
        self.gas_limit
    }
    pub fn max_fee_per_gas(self) -> U256 {
        self.max_fee_per_gas
    }
    pub fn max_priority_fee_per_gas(self) -> U256 {
        self.max_priority_fee_per_gas
    }

    pub fn is_native(self) -> bool {
        self.asset_kind == ASSET_NATIVE
    }

    /// 过期、尚未到可用窗口之外、或系统时间对不上，都拒绝。
    pub fn check_time(self, now: u64) -> Result<(), Error> {
        if self.expiry <= now {
            return Err(Error("简牍已过期"));
        }
        if self.expiry - now > MAX_TTL_SECS {
            return Err(Error("简牍有效期过长"));
        }
        Ok(())
    }

    pub fn encode(self) -> [u8; TABLET_LEN] {
        let mut out = [0u8; TABLET_LEN];
        out[0..4].copy_from_slice(MAGIC);
        out[4] = VERSION;
        out[OFF_FAMILY] = self.family;
        out[OFF_CHAIN..OFF_CHAIN + 8].copy_from_slice(&self.chain_id.to_be_bytes());
        out[OFF_ASSET] = self.asset_kind;
        out[OFF_TOKEN..OFF_TOKEN + 20].copy_from_slice(&self.token);
        out[OFF_TO..OFF_TO + 20].copy_from_slice(&self.recipient);
        out[OFF_AMOUNT..OFF_AMOUNT + 32].copy_from_slice(&self.amount.to_be_bytes());
        out[OFF_FEE_CAP..OFF_FEE_CAP + 32].copy_from_slice(&self.fee_cap.to_be_bytes());
        out[OFF_EXPIRY..OFF_EXPIRY + 8].copy_from_slice(&self.expiry.to_be_bytes());
        out[OFF_NONCE..OFF_NONCE + 8].copy_from_slice(&self.nonce.to_be_bytes());
        out[OFF_GAS..OFF_GAS + 8].copy_from_slice(&self.gas_limit.to_be_bytes());
        out[OFF_MAX_FEE..OFF_MAX_FEE + 32].copy_from_slice(&self.max_fee_per_gas.to_be_bytes());
        out[OFF_PRIORITY..OFF_PRIORITY + 32]
            .copy_from_slice(&self.max_priority_fee_per_gas.to_be_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != TABLET_LEN {
            return Err(Error("简牍长度不对"));
        }
        if &bytes[0..4] != MAGIC {
            return Err(Error("简牍魔数不对"));
        }
        if bytes[4] != VERSION {
            return Err(Error("简牍版本不对"));
        }
        let mut chain = [0u8; 8];
        chain.copy_from_slice(&bytes[OFF_CHAIN..OFF_CHAIN + 8]);
        let mut token = [0u8; 20];
        token.copy_from_slice(&bytes[OFF_TOKEN..OFF_TOKEN + 20]);
        let mut recipient = [0u8; 20];
        recipient.copy_from_slice(&bytes[OFF_TO..OFF_TO + 20]);
        let mut amount = [0u8; 32];
        amount.copy_from_slice(&bytes[OFF_AMOUNT..OFF_AMOUNT + 32]);
        let mut fee_cap = [0u8; 32];
        fee_cap.copy_from_slice(&bytes[OFF_FEE_CAP..OFF_FEE_CAP + 32]);
        let mut expiry = [0u8; 8];
        expiry.copy_from_slice(&bytes[OFF_EXPIRY..OFF_EXPIRY + 8]);
        let mut nonce = [0u8; 8];
        nonce.copy_from_slice(&bytes[OFF_NONCE..OFF_NONCE + 8]);
        let mut gas = [0u8; 8];
        gas.copy_from_slice(&bytes[OFF_GAS..OFF_GAS + 8]);
        let mut max_fee = [0u8; 32];
        max_fee.copy_from_slice(&bytes[OFF_MAX_FEE..OFF_MAX_FEE + 32]);
        let mut priority = [0u8; 32];
        priority.copy_from_slice(&bytes[OFF_PRIORITY..OFF_PRIORITY + 32]);
        let tablet = Self {
            family: bytes[OFF_FAMILY],
            chain_id: u64::from_be_bytes(chain),
            asset_kind: bytes[OFF_ASSET],
            token,
            recipient,
            amount: U256::from_be_bytes(amount),
            fee_cap: U256::from_be_bytes(fee_cap),
            expiry: u64::from_be_bytes(expiry),
            nonce: u64::from_be_bytes(nonce),
            gas_limit: u64::from_be_bytes(gas),
            max_fee_per_gas: U256::from_be_bytes(max_fee),
            max_priority_fee_per_gas: U256::from_be_bytes(priority),
        };
        tablet.check_shape()?;
        Ok(tablet)
    }

    fn check_shape(self) -> Result<(), Error> {
        if self.family != FAMILY_EVM {
            return Err(Error("不认识的家族"));
        }
        if self.chain_id == 0 {
            return Err(Error("链编号为空"));
        }
        match self.asset_kind {
            ASSET_NATIVE => {
                if self.token.iter().any(|b| *b != 0) {
                    return Err(Error("主币转账不能带代币合约"));
                }
            }
            ASSET_ERC20 => {
                if self.token.iter().all(|b| *b == 0) {
                    return Err(Error("代币合约为空"));
                }
            }
            _ => return Err(Error("不认识的资产种类")),
        }
        if self.recipient.iter().all(|b| *b == 0) {
            return Err(Error("收款地址为空"));
        }
        if self.asset_kind == ASSET_ERC20 && self.recipient == self.token {
            return Err(Error("收款地址与代币合约相同"));
        }
        if self.amount.is_zero() {
            return Err(Error("数量为零"));
        }
        if self.gas_limit == 0 || self.gas_limit > MAX_GAS {
            return Err(Error("gas 超出允许范围"));
        }
        if self.max_fee_per_gas.is_zero() {
            return Err(Error("max_fee_per_gas 为零"));
        }
        if self.max_priority_fee_per_gas.cmp(self.max_fee_per_gas) == Ordering::Greater {
            return Err(Error("优先费高于费用上限单价"));
        }
        let product = self
            .max_fee_per_gas
            .checked_mul_u64(self.gas_limit)
            .ok_or(Error("费用乘积溢出"))?;
        if product.cmp(self.fee_cap) == Ordering::Greater {
            return Err(Error("实际费用封顶超过简牍费用上限"));
        }
        if self.expiry == 0 {
            return Err(Error("过期时间为空"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    tablet: Tablet,
    raw_tx: Vec<u8>,
}

impl Receipt {
    pub fn new(tablet: Tablet, raw_tx: Vec<u8>) -> Result<Self, Error> {
        if raw_tx.is_empty() || raw_tx.len() > MAX_RAW_TX {
            return Err(Error("签名交易长度不对"));
        }
        Ok(Self { tablet, raw_tx })
    }

    pub fn tablet(&self) -> Tablet {
        self.tablet
    }

    pub fn raw_tx(&self) -> &[u8] {
        &self.raw_tx
    }

    pub fn encode(&self) -> Vec<u8> {
        let tablet = self.tablet.encode();
        let mut out = Vec::with_capacity(4 + 1 + TABLET_LEN + 2 + self.raw_tx.len());
        out.extend_from_slice(RECEIPT_MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&tablet);
        let len = u16::try_from(self.raw_tx.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&self.raw_tx);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let min = 4 + 1 + TABLET_LEN + 2;
        if bytes.len() < min || bytes.len() > min + MAX_RAW_TX {
            return Err(Error("回执长度不对"));
        }
        if &bytes[0..4] != RECEIPT_MAGIC {
            return Err(Error("回执魔数不对"));
        }
        if bytes[4] != VERSION {
            return Err(Error("回执版本不对"));
        }
        let tablet = Tablet::decode(&bytes[5..5 + TABLET_LEN])?;
        let mut len_bytes = [0u8; 2];
        len_bytes.copy_from_slice(&bytes[5 + TABLET_LEN..5 + TABLET_LEN + 2]);
        let raw_len = u16::from_be_bytes(len_bytes) as usize;
        if raw_len == 0 || raw_len > MAX_RAW_TX {
            return Err(Error("签名交易长度不对"));
        }
        let rest = &bytes[5 + TABLET_LEN + 2..];
        if rest.len() != raw_len {
            return Err(Error("回执尾部长度不对"));
        }
        Ok(Self {
            tablet,
            raw_tx: rest.to_vec(),
        })
    }
}

/// 人类单位换成最小单位。小数位由调用方给出，本函数不查询链。
pub fn parse_decimal_amount(input: &str, decimals: u8) -> Result<U256, Error> {
    if decimals > 36 {
        return Err(Error("小数位过多"));
    }
    let s = input.trim();
    if s.is_empty() || s.starts_with('+') || s.starts_with('-') {
        return Err(Error("数量格式不对"));
    }
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    if whole.is_empty() && frac.is_empty() {
        return Err(Error("数量格式不对"));
    }
    if !whole.is_empty() && !whole.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error("数量格式不对"));
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error("数量格式不对"));
    }
    if frac.len() > decimals as usize {
        return Err(Error("小数位超过该资产的精度"));
    }
    if whole.len() > 60 {
        return Err(Error("数量过大"));
    }
    let mut digits = String::new();
    let trimmed = whole.trim_start_matches('0');
    if !trimmed.is_empty() {
        digits.push_str(trimmed);
    }
    digits.push_str(frac);
    for _ in frac.len()..decimals as usize {
        digits.push('0');
    }
    if digits.is_empty() {
        digits.push('0');
    }
    let amount = U256::from_dec_str(&digits).ok_or(Error("数量过大"))?;
    if amount.is_zero() {
        return Err(Error("数量为零"));
    }
    Ok(amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(byte: u8) -> [u8; 20] {
        [byte; 20]
    }

    fn sample(expiry: u64) -> Tablet {
        let gas = 21_000u64;
        let max_fee = U256::from_u64(1_000_000_000);
        let fee_cap = max_fee.checked_mul_u64(gas).unwrap();
        Tablet::new(
            FAMILY_EVM,
            1,
            ASSET_NATIVE,
            [0u8; 20],
            addr(0x11),
            U256::from_u64(1_000),
            fee_cap,
            expiry,
            7,
            gas,
            max_fee,
            U256::from_u64(1),
        )
        .unwrap()
    }

    #[test]
    fn u256_arith_and_decimal() {
        assert_eq!(
            U256::from_u64(2).checked_mul_u64(3).unwrap(),
            U256::from_u64(6)
        );
        let eth = U256::from_dec_str("1000000000000000000").unwrap();
        assert_eq!(eth.to_dec_string(), "1000000000000000000");
        assert!(U256::from_be_bytes([0xff; 32])
            .checked_mul_u64(2)
            .is_none());
        assert_eq!(parse_decimal_amount("1.5", 18).unwrap().to_dec_string(), {
            let mut s = "15".to_string();
            s.push_str(&"0".repeat(17));
            s
        });
        assert!(parse_decimal_amount("0", 18).is_err());
        assert!(parse_decimal_amount("1.234", 2).is_err());
        assert_eq!(parse_decimal_amount("00.10", 2).unwrap(), parse_decimal_amount("0.10", 2).unwrap());
        assert!(parse_decimal_amount("1e2", 2).is_err());
        assert!(parse_decimal_amount("１", 2).is_err());
    }

    #[test]
    fn roundtrip_is_exact() {
        let tablet = sample(1_700_000_000);
        let bytes = tablet.encode();
        assert_eq!(bytes.len(), TABLET_LEN);
        let back = Tablet::decode(&bytes).unwrap();
        assert_eq!(back, tablet);
        assert_eq!(back.encode(), bytes);
    }

    #[test]
    fn time_window() {
        let now = 1_700_000_000u64;
        let ok = sample(now + 60);
        ok.check_time(now).unwrap();
        assert!(sample(now).check_time(now).is_err());
        assert!(sample(now - 1).check_time(now).is_err());
        assert!(sample(now + MAX_TTL_SECS + 1).check_time(now).is_err());
        sample(now + MAX_TTL_SECS).check_time(now).unwrap();
    }

    #[test]
    fn malicious_tablets_are_refused() {
        let now = 1_700_000_000u64;
        let good = sample(now + 60).encode();

        let mut trailing = good.to_vec();
        trailing.push(0);
        assert!(Tablet::decode(&trailing).is_err());
        assert!(Tablet::decode(&good[..good.len() - 1]).is_err());

        let mut magic = good;
        magic[0] ^= 1;
        assert_eq!(Tablet::decode(&magic).unwrap_err().msg(), "简牍魔数不对");

        let mut version = good;
        version[4] = 2;
        assert!(Tablet::decode(&version).is_err());

        let mut family = good;
        family[OFF_FAMILY] = 9;
        assert_eq!(Tablet::decode(&family).unwrap_err().msg(), "不认识的家族");

        let mut kind = good;
        kind[OFF_ASSET] = 9;
        assert!(Tablet::decode(&kind).is_err());

        let mut token = good;
        token[OFF_TOKEN] = 1;
        assert_eq!(
            Tablet::decode(&token).unwrap_err().msg(),
            "主币转账不能带代币合约"
        );

        let mut erc20 = good;
        erc20[OFF_ASSET] = ASSET_ERC20;
        assert_eq!(Tablet::decode(&erc20).unwrap_err().msg(), "代币合约为空");

        let mut erc20_ok = good;
        erc20_ok[OFF_ASSET] = ASSET_ERC20;
        erc20_ok[OFF_TOKEN] = 0x22;
        Tablet::decode(&erc20_ok).unwrap();

        let mut erc20_self = erc20_ok;
        erc20_self[OFF_TO..OFF_TO + 20].copy_from_slice(&[0x22; 20]);
        erc20_self[OFF_TOKEN..OFF_TOKEN + 20].copy_from_slice(&[0x22; 20]);
        assert!(Tablet::decode(&erc20_self).is_err());

        let mut to = good;
        to[OFF_TO..OFF_TO + 20].fill(0);
        assert!(Tablet::decode(&to).is_err());

        let mut amount = good;
        amount[OFF_AMOUNT..OFF_AMOUNT + 32].fill(0);
        assert_eq!(Tablet::decode(&amount).unwrap_err().msg(), "数量为零");

        let mut gas = good;
        gas[OFF_GAS..OFF_GAS + 8].copy_from_slice(&0u64.to_be_bytes());
        assert!(Tablet::decode(&gas).is_err());

        let mut gas_hi = good;
        gas_hi[OFF_GAS..OFF_GAS + 8].copy_from_slice(&(MAX_GAS + 1).to_be_bytes());
        assert!(Tablet::decode(&gas_hi).is_err());

        let mut priority = good;
        // max fee is 1 gwei at the end of its 32 bytes; set priority to 2 gwei.
        priority[OFF_PRIORITY + 31] = 2;
        priority[OFF_MAX_FEE + 24..OFF_MAX_FEE + 32].copy_from_slice(&1u64.to_be_bytes());
        // The sample's max fee is 1_000_000_000 which is larger than 2. Set priority above it.
        priority[OFF_PRIORITY..OFF_PRIORITY + 32]
            .copy_from_slice(&U256::from_u64(2_000_000_000).to_be_bytes());
        assert_eq!(
            Tablet::decode(&priority).unwrap_err().msg(),
            "优先费高于费用上限单价"
        );

        let mut cap = good;
        cap[OFF_FEE_CAP..OFF_FEE_CAP + 32].copy_from_slice(&U256::from_u64(1).to_be_bytes());
        assert_eq!(
            Tablet::decode(&cap).unwrap_err().msg(),
            "实际费用封顶超过简牍费用上限"
        );

        let mut expiry = good;
        expiry[OFF_EXPIRY..OFF_EXPIRY + 8].fill(0);
        assert!(Tablet::decode(&expiry).is_err());

        assert!(sample(now + 1).check_time(now + 5).is_err());
        assert!(sample(now + MAX_TTL_SECS + 5)
            .check_time(now)
            .is_err());
    }

    #[test]
    fn malicious_receipts_are_refused() {
        let tablet = sample(1_700_000_000);
        let raw = vec![0x02, 0x01, 0x02, 0x03];
        let receipt = Receipt::new(tablet, raw.clone()).unwrap();
        let bytes = receipt.encode();
        let back = Receipt::decode(&bytes).unwrap();
        assert_eq!(back.tablet(), tablet);
        assert_eq!(back.raw_tx(), raw.as_slice());

        let mut trailing = bytes.clone();
        trailing.push(0xff);
        assert_eq!(
            Receipt::decode(&trailing).unwrap_err().msg(),
            "回执尾部长度不对"
        );

        let mut short = bytes.clone();
        short.pop();
        assert!(Receipt::decode(&short).is_err());

        assert!(Receipt::new(tablet, vec![]).is_err());
        assert!(Receipt::new(tablet, vec![0u8; MAX_RAW_TX + 1]).is_err());

        let mut huge = bytes.clone();
        // Claim a raw length that would run past the buffer, without actually attaching it.
        let len_at = 5 + TABLET_LEN;
        huge[len_at] = 0x02;
        huge[len_at + 1] = 0x00;
        assert!(Receipt::decode(&huge).is_err());
    }
}
