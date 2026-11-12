//! 只知道两件事：主币转账，以及 calldata 恰好是 `transfer(address,uint256)` 的代币转账。
//! 签名在清机里完成。这里负责把简牍还原成 EIP-1559 交易，并核对回执里的原始交易没有被掉包。

#![forbid(unsafe_code)]

use sha3::{Digest, Keccak256};
use tablet::{Tablet, U256, ASSET_ERC20, ASSET_NATIVE, FAMILY_EVM};

#[derive(Debug)]
pub struct Error(&'static str);

impl Error {
    pub fn msg(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Error {}

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&out);
    bytes
}

pub fn transfer_selector() -> [u8; 4] {
    let hash = keccak256(b"transfer(address,uint256)");
    [hash[0], hash[1], hash[2], hash[3]]
}

pub fn transfer_calldata(recipient: &[u8; 20], amount: U256) -> Vec<u8> {
    let mut data = Vec::with_capacity(68);
    data.extend_from_slice(&transfer_selector());
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(recipient);
    data.extend_from_slice(&amount.to_be_bytes());
    data
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedTx {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: U256,
    pub max_fee_per_gas: U256,
    pub gas_limit: u64,
    pub to: [u8; 20],
    pub value: U256,
    pub data: Vec<u8>,
    pub y_parity: u8,
    pub r: [u8; 32],
    pub s: [u8; 32],
}

struct Fields {
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: U256,
    max_fee_per_gas: U256,
    gas_limit: u64,
    to: [u8; 20],
    value: U256,
    data: Vec<u8>,
}

fn fields_of(tablet: &Tablet) -> Result<Fields, Error> {
    if tablet.family() != FAMILY_EVM {
        return Err(Error("不是 EVM 简牍"));
    }
    let (to, value, data) = match tablet.asset_kind() {
        ASSET_NATIVE => (tablet.recipient(), tablet.amount(), Vec::new()),
        ASSET_ERC20 => (
            tablet.token(),
            U256::zero(),
            transfer_calldata(&tablet.recipient(), tablet.amount()),
        ),
        _ => return Err(Error("不认识的资产种类")),
    };
    Ok(Fields {
        chain_id: tablet.chain_id(),
        nonce: tablet.nonce(),
        max_priority_fee_per_gas: tablet.max_priority_fee_per_gas(),
        max_fee_per_gas: tablet.max_fee_per_gas(),
        gas_limit: tablet.gas_limit(),
        to,
        value,
        data,
    })
}

pub struct UnsignedTx {
    pub bytes: Vec<u8>,
    pub sighash: [u8; 32],
}

pub fn unsigned_tx(tablet: &Tablet) -> Result<UnsignedTx, Error> {
    let fields = fields_of(tablet)?;
    let body = rlp_list(&unsigned_payload(&fields));
    let mut bytes = Vec::with_capacity(1 + body.len());
    bytes.push(0x02);
    bytes.extend_from_slice(&body);
    let sighash = keccak256(&bytes);
    Ok(UnsignedTx { bytes, sighash })
}

pub fn assemble(tablet: &Tablet, y_parity: u8, r: [u8; 32], s: [u8; 32]) -> Result<Vec<u8>, Error> {
    if y_parity > 1 || r == [0u8; 32] || s == [0u8; 32] {
        return Err(Error("签名无效"));
    }
    let fields = fields_of(tablet)?;
    let mut payload = unsigned_payload(&fields);
    payload.extend(rlp_u64(y_parity as u64));
    payload.extend(rlp_u256(&U256::from_be_bytes(r)));
    payload.extend(rlp_u256(&U256::from_be_bytes(s)));
    let body = rlp_list(&payload);
    let mut raw = Vec::with_capacity(1 + body.len());
    raw.push(0x02);
    raw.extend_from_slice(&body);
    if raw.len() > tablet::MAX_RAW_TX {
        return Err(Error("签名交易过长"));
    }
    Ok(raw)
}

pub fn decode_raw(raw: &[u8]) -> Result<DecodedTx, Error> {
    if raw.first() != Some(&0x02) {
        return Err(Error("不是 EIP-1559 交易"));
    }
    let (value, used) = parse_value(&raw[1..])?;
    if used != raw.len() - 1 {
        return Err(Error("交易尾部还有数据"));
    }
    let Value::List(items) = value else {
        return Err(Error("交易不是列表"));
    };
    if items.len() != 12 {
        return Err(Error("交易字段数量不对"));
    }
    if !matches!(&items[8], Value::List(inner) if inner.is_empty()) {
        return Err(Error("访问列表必须为空"));
    }
    let chain_id = as_u64(&items[0])?;
    let nonce = as_u64(&items[1])?;
    let max_priority_fee_per_gas = as_u256(&items[2])?;
    let max_fee_per_gas = as_u256(&items[3])?;
    let gas_limit = as_u64(&items[4])?;
    let to = as_addr(&items[5])?;
    let value = as_u256(&items[6])?;
    let data = as_data(&items[7])?;
    let y_parity_u = as_u64(&items[9])?;
    if y_parity_u > 1 {
        return Err(Error("签名无效"));
    }
    let r = as_padded_32(&items[10])?;
    let s = as_padded_32(&items[11])?;
    if r == [0u8; 32] || s == [0u8; 32] {
        return Err(Error("签名无效"));
    }
    Ok(DecodedTx {
        chain_id,
        nonce,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        gas_limit,
        to,
        value,
        data,
        y_parity: y_parity_u as u8,
        r,
        s,
    })
}

pub fn raw_matches_tablet(raw: &[u8], tablet: &Tablet) -> Result<(), Error> {
    let decoded = decode_raw(raw)?;
    let fields = fields_of(tablet)?;
    if decoded.chain_id != fields.chain_id
        || decoded.nonce != fields.nonce
        || decoded.max_priority_fee_per_gas != fields.max_priority_fee_per_gas
        || decoded.max_fee_per_gas != fields.max_fee_per_gas
        || decoded.gas_limit != fields.gas_limit
        || decoded.to != fields.to
        || decoded.value != fields.value
        || decoded.data != fields.data
    {
        return Err(Error("回执交易与简牍不一致"));
    }
    Ok(())
}

fn unsigned_payload(fields: &Fields) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend(rlp_u64(fields.chain_id));
    payload.extend(rlp_u64(fields.nonce));
    payload.extend(rlp_u256(&fields.max_priority_fee_per_gas));
    payload.extend(rlp_u256(&fields.max_fee_per_gas));
    payload.extend(rlp_u64(fields.gas_limit));
    payload.extend(rlp_bytes(&fields.to));
    payload.extend(rlp_u256(&fields.value));
    payload.extend(rlp_bytes(&fields.data));
    payload.extend(rlp_list(&[]));
    payload
}

fn rlp_u64(n: u64) -> Vec<u8> {
    rlp_u256(&U256::from_u64(n))
}

fn rlp_u256(n: &U256) -> Vec<u8> {
    let bytes = n.to_be_bytes();
    let start = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
    rlp_bytes(&bytes[start..])
}

fn rlp_bytes(data: &[u8]) -> Vec<u8> {
    if data.len() == 1 && data[0] < 0x80 {
        return vec![data[0]];
    }
    let mut out = rlp_header(0x80, data.len());
    out.extend_from_slice(data);
    out
}

fn rlp_list(payload: &[u8]) -> Vec<u8> {
    let mut out = rlp_header(0xc0, payload.len());
    out.extend_from_slice(payload);
    out
}

fn rlp_header(offset: u8, len: usize) -> Vec<u8> {
    if len < 56 {
        vec![offset + len as u8]
    } else {
        let be = len.to_be_bytes();
        let start = be.iter().position(|b| *b != 0).unwrap_or(be.len() - 1);
        let bytes = &be[start..];
        let mut out = Vec::with_capacity(1 + bytes.len());
        out.push(offset + 55 + bytes.len() as u8);
        out.extend_from_slice(bytes);
        out
    }
}

enum Value {
    Bytes(Vec<u8>),
    List(Vec<Value>),
}

fn parse_value(input: &[u8]) -> Result<(Value, usize), Error> {
    if input.is_empty() {
        return Err(Error("交易被截断"));
    }
    let b = input[0];
    if b < 0x80 {
        return Ok((Value::Bytes(vec![b]), 1));
    }
    if b <= 0xb7 {
        let len = (b - 0x80) as usize;
        let end = 1 + len;
        if input.len() < end {
            return Err(Error("交易被截断"));
        }
        let data = input[1..end].to_vec();
        if len == 1 && data[0] < 0x80 {
            return Err(Error("非规范的 RLP"));
        }
        return Ok((Value::Bytes(data), end));
    }
    if b <= 0xbf {
        return parse_long(input, b - 0xb7, false);
    }
    if b <= 0xf7 {
        let len = (b - 0xc0) as usize;
        let end = 1 + len;
        if input.len() < end {
            return Err(Error("交易被截断"));
        }
        let items = parse_sequence(&input[1..end])?;
        return Ok((Value::List(items), end));
    }
    parse_long(input, b - 0xf7, true)
}

fn parse_long(input: &[u8], len_of_len: u8, list: bool) -> Result<(Value, usize), Error> {
    let ll = len_of_len as usize;
    if ll == 0 || ll > 8 {
        return Err(Error("非规范的 RLP"));
    }
    if input.len() < 1 + ll {
        return Err(Error("交易被截断"));
    }
    if input[1] == 0 {
        return Err(Error("非规范的 RLP"));
    }
    let mut n: usize = 0;
    for i in 0..ll {
        n = (n << 8) | input[1 + i] as usize;
    }
    if n < 56 {
        return Err(Error("非规范的 RLP"));
    }
    let end = 1 + ll + n;
    if input.len() < end {
        return Err(Error("交易被截断"));
    }
    if list {
        let items = parse_sequence(&input[1 + ll..end])?;
        Ok((Value::List(items), end))
    } else {
        Ok((Value::Bytes(input[1 + ll..end].to_vec()), end))
    }
}

fn parse_sequence(mut payload: &[u8]) -> Result<Vec<Value>, Error> {
    let mut items = Vec::new();
    while !payload.is_empty() {
        let (item, used) = parse_value(payload)?;
        items.push(item);
        payload = &payload[used..];
    }
    Ok(items)
}

fn as_bytes(value: &Value) -> Result<&[u8], Error> {
    match value {
        Value::Bytes(bytes) => Ok(bytes),
        Value::List(_) => Err(Error("交易字段类型不对")),
    }
}

fn as_u256(value: &Value) -> Result<U256, Error> {
    let bytes = as_bytes(value)?;
    if bytes.len() > 32 {
        return Err(Error("整数过长"));
    }
    if bytes.is_empty() {
        return Ok(U256::zero());
    }
    if bytes[0] == 0 {
        return Err(Error("非规范整数"));
    }
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(bytes);
    Ok(U256::from_be_bytes(out))
}

fn as_u64(value: &Value) -> Result<u64, Error> {
    as_u256(value)?
        .to_u64()
        .ok_or(Error("整数超出范围"))
}

fn as_addr(value: &Value) -> Result<[u8; 20], Error> {
    let bytes = as_bytes(value)?;
    if bytes.len() != 20 {
        return Err(Error("地址长度不对"));
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(bytes);
    Ok(out)
}

fn as_data(value: &Value) -> Result<Vec<u8>, Error> {
    Ok(as_bytes(value)?.to_vec())
}

fn as_padded_32(value: &Value) -> Result<[u8; 32], Error> {
    let n = as_u256(value)?;
    Ok(n.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tablet::{Receipt, ASSET_ERC20, FAMILY_EVM, MAX_TTL_SECS};

    fn addr(byte: u8) -> [u8; 20] {
        [byte; 20]
    }

    fn tablet(kind: u8, token: [u8; 20]) -> Tablet {
        let gas = if kind == ASSET_NATIVE { 21_000 } else { 120_000 };
        let max_fee = U256::from_u64(2_000_000_000);
        let fee_cap = max_fee.checked_mul_u64(gas).unwrap();
        Tablet::new(
            FAMILY_EVM,
            8453,
            kind,
            token,
            addr(0xab),
            U256::from_u64(5),
            fee_cap,
            1_700_000_000 + MAX_TTL_SECS,
            3,
            gas,
            max_fee,
            U256::from_u64(1_000_000_000),
        )
        .unwrap()
    }

    #[test]
    fn keccak_and_selector() {
        assert_eq!(
            keccak256(b""),
            hex_bytes("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
        );
        assert_eq!(transfer_selector(), [0xa9, 0x05, 0x9c, 0xbb]);
        assert_ne!(&transfer_selector(), &[0x09, 0x5e, 0xa7, 0xb3]);
    }

    #[test]
    fn rlp_known_vectors() {
        assert_eq!(rlp_bytes(b""), vec![0x80]);
        assert_eq!(rlp_bytes(b"dog"), vec![0x83, b'd', b'o', b'g']);
        assert_eq!(rlp_u64(0), vec![0x80]);
        assert_eq!(rlp_u64(15), vec![0x0f]);
        assert_eq!(rlp_u64(1024), vec![0x82, 0x04, 0x00]);
        assert_eq!(rlp_list(&[]), vec![0xc0]);
        let mut list = rlp_bytes(b"cat");
        list.extend(rlp_bytes(b"dog"));
        assert_eq!(
            rlp_list(&list),
            vec![0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g']
        );
    }

    #[test]
    fn native_and_erc20_roundtrip_and_reject_swap() {
        let native = tablet(ASSET_NATIVE, [0u8; 20]);
        let raw = assemble(&native, 0, pad(1), pad(2)).unwrap();
        raw_matches_tablet(&raw, &native).unwrap();
        let decoded = decode_raw(&raw).unwrap();
        assert!(decoded.data.is_empty());
        assert_eq!(decoded.value, native.amount());
        assert_eq!(decoded.to, native.recipient());
        assert!(decoded.data.windows(4).all(|w| w != [0x09, 0x5e, 0xa7, 0xb3]));

        let erc20 = tablet(ASSET_ERC20, addr(0x44));
        let raw = assemble(&erc20, 1, pad(3), pad(4)).unwrap();
        raw_matches_tablet(&raw, &erc20).unwrap();
        let decoded = decode_raw(&raw).unwrap();
        assert_eq!(decoded.to, erc20.token());
        assert!(decoded.value.is_zero());
        assert_eq!(
            decoded.data,
            transfer_calldata(&erc20.recipient(), erc20.amount())
        );
        assert_eq!(&decoded.data[..4], &[0xa9, 0x05, 0x9c, 0xbb]);

        let mut swapped = raw.clone();
        swapped[10] ^= 0x01;
        assert!(raw_matches_tablet(&swapped, &erc20).is_err());
        assert!(decode_raw(&[0x02, 0xc2, 0x81, 0x01, 0x80]).is_err());
        let mut padded = vec![0x02, 0xce, 0x82, 0x00, 0x01];
        padded.extend(std::iter::repeat(0x80).take(11));
        assert!(decode_raw(&padded).is_err());

        let receipt = Receipt::new(erc20, raw).unwrap();
        let bytes = receipt.encode();
        assert_eq!(Receipt::decode(&bytes).unwrap().tablet(), erc20);
    }

    fn pad(byte: u8) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[31] = byte;
        out
    }

    fn hex_bytes(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }
}
