use bip32::{DerivationPath, XPrv};
use bip39::Mnemonic;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::SecretKey;
use zeroize::{Zeroize, Zeroizing};

use crate::Error;
use konghu_evm::keccak256;

const EVM_PATH: &str = "m/44'/60'/0'/0/0";

pub fn mnemonic_from_entropy(entropy: &[u8; 16]) -> Result<Zeroizing<String>, Error> {
    let mnemonic = Mnemonic::from_entropy(entropy).map_err(|_| Error::Key("熵无效"))?;
    Ok(Zeroizing::new(mnemonic.to_string()))
}

pub fn secret_from_entropy(entropy: &[u8; 16]) -> Result<Zeroizing<[u8; 32]>, Error> {
    let mnemonic = Mnemonic::from_entropy(entropy).map_err(|_| Error::Key("熵无效"))?;
    let mut seed = mnemonic.to_seed("");
    let secret = secret_from_seed(&seed)?;
    seed.zeroize();
    Ok(secret)
}

fn secret_from_seed(seed: &[u8]) -> Result<Zeroizing<[u8; 32]>, Error> {
    let path: DerivationPath = EVM_PATH.parse().map_err(|_| Error::Key("派生路径无效"))?;
    let child = XPrv::derive_from_path(seed, &path).map_err(|_| Error::Key("无法从种子派生"))?;
    let bytes = child.private_key().to_bytes();
    let mut secret = Zeroizing::new([0u8; 32]);
    secret.copy_from_slice(&bytes);
    Ok(secret)
}

pub fn address_from_secret(secret: &[u8; 32]) -> Result<[u8; 20], Error> {
    let sk = SecretKey::from_slice(secret).map_err(|_| Error::Key("私钥无效"))?;
    let point = sk.public_key().to_encoded_point(false);
    let hash = keccak256(&point.as_bytes()[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    Ok(address)
}

pub fn eip55(address: &[u8; 20]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bare = String::with_capacity(40);
    for byte in address {
        bare.push(HEX[(byte >> 4) as usize] as char);
        bare.push(HEX[(byte & 0x0f) as usize] as char);
    }
    let hash = keccak256(bare.as_bytes());
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (i, ch) in bare.chars().enumerate() {
        let nibble = if i % 2 == 0 {
            hash[i / 2] >> 4
        } else {
            hash[i / 2] & 0x0f
        };
        if ch.is_ascii_alphabetic() && nibble >= 8 {
            out.push(ch.to_ascii_uppercase());
        } else {
            out.push(ch);
        }
    }
    out
}
