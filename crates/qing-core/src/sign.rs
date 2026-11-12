use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use tablet::Tablet;

use crate::hd::{address_from_secret, eip55};
use crate::Error;
use konghu_evm::{self, keccak256};

pub fn confirmation_phrase(tablet: &Tablet) -> Result<String, Error> {
    let product = fee_product(tablet)?;
    let mut phrase = format!(
        "{}/{}/{}",
        tablet.chain_id(),
        tablet.amount().to_dec_string(),
        product.to_dec_string()
    );
    if tablet.asset_kind() == tablet::ASSET_ERC20 {
        phrase.push('/');
        phrase.push_str(&hex40(&tablet.token()));
    }
    phrase.push('/');
    phrase.push_str(&hex40(&tablet.recipient()));
    Ok(phrase)
}

pub fn confirmation_matches(tablet: &Tablet, typed: &str) -> bool {
    match confirmation_phrase(tablet) {
        Ok(phrase) => phrase.eq_ignore_ascii_case(typed.trim()),
        Err(_) => false,
    }
}

pub fn accept_note(note: &str) -> Option<&str> {
    let note = note.trim();
    if note.is_empty() || note.chars().count() > 32 || note.chars().any(|ch| !note_char_ok(ch)) {
        None
    } else {
        Some(note)
    }
}

fn note_char_ok(ch: char) -> bool {
    if ch.is_control() || ch == '\u{2028}' || ch == '\u{2029}' {
        return false;
    }
    let u = ch as u32;
    if (0x200B..=0x200F).contains(&u)
        || (0x202A..=0x202E).contains(&u)
        || (0x2060..=0x206F).contains(&u)
        || u == 0xFEFF
        || u == 0x061C
        || u == 0x180E
        || u == 0x00AD
        || (0xFE00..=0xFE0F).contains(&u)
        || (0xFFF9..=0xFFFB).contains(&u)
    {
        return false;
    }
    true
}

fn fee_product(tablet: &Tablet) -> Result<tablet::U256, Error> {
    tablet
        .max_fee_per_gas()
        .checked_mul_u64(tablet.gas_limit())
        .ok_or(Error::Tablet(tablet::Error::new("费用乘积溢出")))
}

fn hex40(bytes: &[u8; 20]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(40);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn inspect(
    tablet: &Tablet,
    now: u64,
    from: Option<&[u8; 20]>,
    label: &dyn Fn(&[u8; 20]) -> Option<String>,
) -> Result<String, Error> {
    tablet.check_time(now).map_err(Error::Tablet)?;
    let product = tablet
        .max_fee_per_gas()
        .checked_mul_u64(tablet.gas_limit())
        .ok_or(Error::Tablet(tablet::Error::new("费用乘积溢出")))?;
    let mut text = String::new();
    text.push_str("家族：EVM\r\n");
    text.push_str(&format!("链编号：{}\r\n", tablet.chain_id()));
    if let Some(from) = from {
        text.push_str(&format!("付款地址：{}\r\n", eip55(from)));
        push_note(&mut text, "付款备注：", label(from));
    }
    text.push_str(&format!("收款地址：{}\r\n", eip55(&tablet.recipient())));
    push_note(&mut text, "收款备注：", label(&tablet.recipient()));
    match tablet.asset_kind() {
        tablet::ASSET_NATIVE => text.push_str("资产：主币\r\n"),
        tablet::ASSET_ERC20 => {
            text.push_str("资产：代币\r\n");
            text.push_str(&format!("代币合约：{}\r\n", eip55(&tablet.token())));
            push_note(&mut text, "代币备注：", label(&tablet.token()));
        }
        _ => return Err(Error::Tablet(tablet::Error::new("不认识的资产种类"))),
    }
    text.push_str(&format!(
        "数量（最小单位）：{}\r\n",
        tablet.amount().to_dec_string()
    ));
    text.push_str(&format!("费用上限（最小单位）：{}\r\n", tablet.fee_cap().to_dec_string()));
    text.push_str(&format!("实际封顶（最小单位）：{}\r\n", product.to_dec_string()));
    text.push_str(&format!("nonce：{}\r\n", tablet.nonce()));
    text.push_str(&format!("gas_limit：{}\r\n", tablet.gas_limit()));
    text.push_str(&format!(
        "max_fee_per_gas：{}\r\n",
        tablet.max_fee_per_gas().to_dec_string()
    ));
    text.push_str(&format!(
        "max_priority_fee_per_gas：{}\r\n",
        tablet.max_priority_fee_per_gas().to_dec_string()
    ));
    text.push_str(&format!("过期（unix 秒）：{}\r\n", tablet.expiry()));
    text.push_str("按自己的记录输入确认串：链编号/数量/实际封顶/收款地址。代币再在封顶后加上合约地址。不要照着本窗口抄。清机不显示链上名称，也不换算小数位。\r\n");
    Ok(text)
}

fn push_note(text: &mut String, title: &str, note: Option<String>) {
    let Some(note) = note.as_deref().and_then(accept_note) else {
        return;
    };
    text.push_str(title);
    text.push_str(note);
    text.push_str("\r\n");
}

pub fn sign_tablet(
    tablet: &Tablet,
    secret: &[u8; 32],
    now: u64,
) -> Result<tablet::Receipt, Error> {
    tablet.check_time(now).map_err(Error::Tablet)?;
    let unsigned = konghu_evm::unsigned_tx(tablet)?;
    let signing = SigningKey::from_slice(secret).map_err(|_| Error::Key("私钥无效"))?;
    let (signature, recid) = signing
        .sign_prehash_recoverable(&unsigned.sighash)
        .map_err(|_| Error::Key("签名失败"))?;
    let y = recid.to_byte();
    let compact = signature.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&compact[..32]);
    s.copy_from_slice(&compact[32..]);
    let raw = konghu_evm::assemble(tablet, y, r, s)?;
    konghu_evm::raw_matches_tablet(&raw, tablet)?;
    let from = address_from_secret(secret)?;
    let recovered = recover_address(&unsigned.sighash, y, &r, &s)?;
    if recovered != from {
        return Err(Error::Key("签名与付款地址不一致"));
    }
    tablet::Receipt::new(*tablet, raw).map_err(Error::Tablet)
}

fn recover_address(sighash: &[u8; 32], y: u8, r: &[u8; 32], s: &[u8; 32]) -> Result<[u8; 20], Error> {
    let mut compact = [0u8; 64];
    compact[..32].copy_from_slice(r);
    compact[32..].copy_from_slice(s);
    let signature = Signature::from_slice(&compact).map_err(|_| Error::Key("签名无效"))?;
    let recid = RecoveryId::try_from(y).map_err(|_| Error::Key("签名无效"))?;
    let key = VerifyingKey::recover_from_prehash(sighash, &signature, recid)
        .map_err(|_| Error::Key("签名无效"))?;
    let point = key.to_encoded_point(false);
    let hash = keccak256(&point.as_bytes()[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    Ok(address)
}
