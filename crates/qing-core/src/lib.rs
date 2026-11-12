//! 清机核心。不做网络请求，不读取链登记表。

#![forbid(unsafe_code)]

mod hd;
mod sign;
mod vault;

pub use hd::{address_from_secret, eip55, mnemonic_from_entropy, secret_from_entropy};
pub use sign::{accept_note, confirmation_matches, confirmation_phrase, inspect, sign_tablet};
pub use vault::{open, random_entropy, seal, M_KIB, P_COST, T_COST, VAULT_LEN};

#[derive(Debug)]
pub enum Error {
    Tablet(tablet::Error),
    Evm(konghu_evm::Error),
    Vault(&'static str),
    Key(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Tablet(err) => write!(f, "{err}"),
            Error::Evm(err) => write!(f, "{err}"),
            Error::Vault(msg) | Error::Key(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for Error {}

impl From<konghu_evm::Error> for Error {
    fn from(value: konghu_evm::Error) -> Self {
        Error::Evm(value)
    }
}

pub fn now_unix() -> Result<u64, Error> {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| Error::Key("系统时间不可用"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tablet::{Tablet, U256, ASSET_ERC20, ASSET_NATIVE, FAMILY_EVM};

    fn sample(kind: u8, token: [u8; 20], expiry: u64) -> Tablet {
        let gas = if kind == ASSET_NATIVE { 21_000 } else { 100_000 };
        let max_fee = U256::from_u64(3_000_000_000);
        let fee_cap = max_fee.checked_mul_u64(gas).unwrap();
        Tablet::new(
            FAMILY_EVM,
            1,
            kind,
            token,
            [0x22; 20],
            U256::from_dec_str("1000000000000000000").unwrap(),
            fee_cap,
            expiry,
            0,
            gas,
            max_fee,
            U256::from_u64(1_000_000_000),
        )
        .unwrap()
    }

    #[test]
    fn hardhat_address_and_sign_recovers() {
        let secret = hex32("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");
        let address = address_from_secret(&secret).unwrap();
        assert_eq!(
            eip55(&address),
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        );
        let now = 1_700_000_000u64;
        let tablet = sample(ASSET_NATIVE, [0u8; 20], now + 600);
        let receipt = sign_tablet(&tablet, &secret, now).unwrap();
        konghu_evm::raw_matches_tablet(receipt.raw_tx(), &tablet).unwrap();
        let decoded = konghu_evm::decode_raw(receipt.raw_tx()).unwrap();
        assert!(decoded.data.is_empty());

        let token = [0x33; 20];
        let erc20 = sample(ASSET_ERC20, token, now + 600);
        let receipt = sign_tablet(&erc20, &secret, now).unwrap();
        let decoded = konghu_evm::decode_raw(receipt.raw_tx()).unwrap();
        assert_eq!(&decoded.data[..4], &[0xa9, 0x05, 0x9c, 0xbb]);
        assert!(decoded.value.is_zero());
        assert_eq!(decoded.to, token);
        assert!(decoded
            .data
            .windows(4)
            .all(|w| w != [0x09, 0x5e, 0xa7, 0xb3]));

        assert!(sign_tablet(&tablet, &secret, now + 10_000).is_err());
        let far = sample(ASSET_NATIVE, [0u8; 20], now + tablet::MAX_TTL_SECS + 10);
        assert!(sign_tablet(&far, &secret, now).is_err());
    }

    #[test]
    fn inspect_has_no_remote_name() {
        let now = 1_700_000_000u64;
        let tablet = sample(ASSET_ERC20, [0x44; 20], now + 60);
        let text = inspect(&tablet, now, None, &|addr| {
            if addr == &[0x22; 20] {
                Some("自己的备注".to_string())
            } else {
                None
            }
        })
        .unwrap();
        assert!(text.contains("链编号：1"));
        assert!(text.contains("收款备注：自己的备注"));
        assert!(text.contains(&eip55(&[0x44; 20])));
        assert!(!text.contains("USDC"));
        assert!(!text.contains("http"));
        let phrase = confirmation_phrase(&tablet).unwrap();
        assert!(phrase.contains(&hex40(&[0x22; 20])));
        assert!(phrase.contains(&hex40(&[0x44; 20])));
        assert!(confirmation_matches(&tablet, &phrase.to_uppercase()));
        assert!(!confirmation_matches(&tablet, "000000"));
        assert!(text.contains("链编号/数量/实际封顶/收款地址"));
        assert!(!text.contains('\u{202E}'));
    }

    #[test]
    fn attack_known_suffix_rejects_a_different_tablet() {
        let now = 1_700_000_000u64;
        let friend = [0x11u8; 20];
        let mut lookalike = [0x99u8; 20];
        lookalike[17] = 0x11;
        lookalike[18] = 0x11;
        lookalike[19] = 0x11;
        let honest = pay(friend, 1, now);
        let inflated = pay(friend, 1_000_000_000_000_000_000u64, now);
        let swapped = pay(lookalike, 1, now);
        let dearer = tablet::Tablet::new(
            tablet::FAMILY_EVM,
            1,
            tablet::ASSET_NATIVE,
            [0u8; 20],
            friend,
            tablet::U256::from_u64(1),
            tablet::U256::from_u64(21_000 * 9),
            now + 60,
            0,
            21_000,
            tablet::U256::from_u64(9),
            tablet::U256::zero(),
        )
        .unwrap();
        let old_suffix = hex40(&friend)[34..].to_string();
        let typed = confirmation_phrase(&honest).unwrap();
        assert_ne!(honest.encode(), inflated.encode());
        assert_ne!(honest.recipient(), swapped.recipient());
        assert!(!confirmation_matches(&inflated, &old_suffix));
        assert!(!confirmation_matches(&swapped, &old_suffix));
        assert!(!confirmation_matches(&inflated, &typed));
        assert!(!confirmation_matches(&swapped, &typed));
        assert!(!confirmation_matches(&dearer, &typed));
        assert!(confirmation_matches(&honest, &typed));
        let spoofed = inspect(&inflated, now, None, &|addr| {
            if addr == friend.as_slice() {
                Some("\u{202E}交易所".to_string())
            } else {
                None
            }
        })
        .unwrap();
        assert!(!spoofed.contains('\u{202E}'));
        assert!(spoofed.contains("数量（最小单位）"));
        assert!(accept_note("\u{202E}交易所").is_none());
        assert_eq!(accept_note(" 交易所 "), Some("交易所"));
        for hidden in ["\u{E0020}交易所", "\u{3164}交易所", "\u{115F}交易所"] {
            assert!(accept_note(hidden).is_some());
            let shown = inspect(&inflated, now, None, &|addr| {
                if addr == friend.as_slice() {
                    Some(hidden.to_string())
                } else {
                    None
                }
            })
            .unwrap();
            let mark = shown.find(hidden.chars().next().unwrap()).unwrap();
            let amount_at = shown.find("数量（最小单位）").unwrap();
            assert!(mark < amount_at);
        }
    }

    #[test]
    fn attack_same_phrase_signs_a_second_payment() {
        let now = 1_700_000_000u64;
        let friend = [0x11u8; 20];
        let secret = hex32("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");
        let first = shaped(friend, 1, now + 60, 0, 21_000, 2, 0);
        let second = shaped(friend, 1, now + 60, 9, 21_000, 2, 0);
        let later = shaped(friend, 1, now + 120, 0, 21_000, 2, 0);
        let other_gas = shaped(friend, 1, now + 60, 0, 42_000, 1, 0);
        let higher_tip = shaped(friend, 1, now + 60, 0, 21_000, 2, 2);
        let typed = confirmation_phrase(&first).unwrap();
        assert_ne!(first.encode(), second.encode());
        assert_eq!(confirmation_phrase(&other_gas).unwrap(), typed);
        assert!(confirmation_matches(&second, &typed));
        assert!(confirmation_matches(&later, &typed));
        assert!(confirmation_matches(&other_gas, &typed));
        assert!(confirmation_matches(&higher_tip, &typed));
        let first_receipt = sign_tablet(&first, &secret, now).unwrap();
        let second_receipt = sign_tablet(&second, &secret, now).unwrap();
        assert_ne!(first_receipt.raw_tx(), second_receipt.raw_tx());
        konghu_evm::raw_matches_tablet(second_receipt.raw_tx(), &second).unwrap();
        assert!(konghu_evm::raw_matches_tablet(second_receipt.raw_tx(), &first).is_err());

        let padded = typed.replacen("/1/", "/01/", 1);
        assert_ne!(padded, typed);
        assert!(!confirmation_matches(&first, &padded));
        assert!(!confirmation_matches(&first, &typed.replace('1', "\u{FF11}")));
        assert!(!confirmation_matches(&first, &format!("{typed}/")));
        let token = [0x44u8; 20];
        let erc20 = tablet::Tablet::new(
            tablet::FAMILY_EVM,
            1,
            tablet::ASSET_ERC20,
            token,
            friend,
            tablet::U256::from_u64(1),
            tablet::U256::from_u64(120_000),
            now + 60,
            0,
            120_000,
            tablet::U256::from_u64(1),
            tablet::U256::zero(),
        )
        .unwrap();
        assert!(!confirmation_matches(&erc20, &typed));
        assert!(!confirmation_matches(&shaped([0x22; 20], 1, now + 60, 0, 21_000, 2, 0), &typed.to_uppercase()));
        assert!(confirmation_matches(&first, &typed.to_uppercase()));
    }

    fn shaped(
        recipient: [u8; 20],
        amount: u64,
        expiry: u64,
        nonce: u64,
        gas: u64,
        max_fee: u64,
        priority: u64,
    ) -> tablet::Tablet {
        let product = tablet::U256::from_u64(max_fee).checked_mul_u64(gas).unwrap();
        tablet::Tablet::new(
            tablet::FAMILY_EVM,
            1,
            tablet::ASSET_NATIVE,
            [0u8; 20],
            recipient,
            tablet::U256::from_u64(amount),
            product,
            expiry,
            nonce,
            gas,
            tablet::U256::from_u64(max_fee),
            tablet::U256::from_u64(priority),
        )
        .unwrap()
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

    fn pay(recipient: [u8; 20], amount: u64, now: u64) -> tablet::Tablet {
        tablet::Tablet::new(
            tablet::FAMILY_EVM,
            1,
            tablet::ASSET_NATIVE,
            [0u8; 20],
            recipient,
            tablet::U256::from_u64(amount),
            tablet::U256::from_u64(21_000),
            now + 60,
            0,
            21_000,
            tablet::U256::from_u64(1),
            tablet::U256::zero(),
        )
        .unwrap()
    }

    #[test]
    fn mnemonic_roundtrip_derives_same_key() {
        let entropy = [0x42; 16];
        let words = mnemonic_from_entropy(&entropy).unwrap();
        assert_eq!(words.split_whitespace().count(), 12);
        let secret = secret_from_entropy(&entropy).unwrap();
        let again = secret_from_entropy(&entropy).unwrap();
        assert_eq!(secret.as_slice(), again.as_slice());
        address_from_secret(&secret).unwrap();
    }

    fn hex32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }
}
