//! 口令只加密本机的熵。它不是助记词的第 25 个词。
//! 参数写死：换一份保险库不能要求更大的内存，也不能把派生改弱。

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use zeroize::{Zeroize, Zeroizing};

use crate::Error;

pub const MAGIC: &[u8; 4] = b"KHUV";
const VERSION: u8 = 1;
pub const M_KIB: u32 = 19 * 1024;
pub const T_COST: u32 = 2;
pub const P_COST: u32 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const PLAIN_LEN: usize = 16;
const TAG_LEN: usize = 16;
pub const VAULT_LEN: usize = 4 + 1 + 4 + 4 + 4 + SALT_LEN + NONCE_LEN + PLAIN_LEN + TAG_LEN;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Kdf {
    m_kib: u32,
    t: u32,
    p: u32,
}

fn production() -> Kdf {
    Kdf {
        m_kib: M_KIB,
        t: T_COST,
        p: P_COST,
    }
}

pub fn seal(password: &[u8], entropy: &[u8; 16]) -> Result<Vec<u8>, Error> {
    seal_with(password, entropy, production())
}

pub fn open(bytes: &[u8], password: &[u8]) -> Result<Zeroizing<[u8; 16]>, Error> {
    let kdf = read_kdf(bytes)?;
    if kdf != production() {
        return Err(Error::Vault("保险库口令参数不对"));
    }
    decrypt(bytes, password, kdf)
}

fn seal_with(password: &[u8], entropy: &[u8; 16], kdf: Kdf) -> Result<Vec<u8>, Error> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut salt).map_err(|_| Error::Vault("无法读取系统随机数"))?;
    getrandom::getrandom(&mut nonce).map_err(|_| Error::Vault("无法读取系统随机数"))?;
    let mut key = derive_key(password, &salt, kdf)?;
    let cipher = ChaCha20Poly1305::new_from_slice(key.as_slice())
        .map_err(|_| Error::Vault("保险库封装失败"))?;
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), entropy.as_slice())
        .map_err(|_| Error::Vault("保险库封装失败"))?;
    key.zeroize();
    if encrypted.len() != PLAIN_LEN + TAG_LEN {
        return Err(Error::Vault("保险库封装失败"));
    }
    let mut out = Vec::with_capacity(VAULT_LEN);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&kdf.m_kib.to_be_bytes());
    out.extend_from_slice(&kdf.t.to_be_bytes());
    out.extend_from_slice(&kdf.p.to_be_bytes());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&encrypted);
    debug_assert_eq!(out.len(), VAULT_LEN);
    Ok(out)
}

fn read_kdf(bytes: &[u8]) -> Result<Kdf, Error> {
    if bytes.len() != VAULT_LEN {
        return Err(Error::Vault("保险库长度不对"));
    }
    if &bytes[0..4] != MAGIC {
        return Err(Error::Vault("不是保险库文件"));
    }
    if bytes[4] != VERSION {
        return Err(Error::Vault("保险库版本不对"));
    }
    let m_kib = u32::from_be_bytes(bytes[5..9].try_into().unwrap());
    let t = u32::from_be_bytes(bytes[9..13].try_into().unwrap());
    let p = u32::from_be_bytes(bytes[13..17].try_into().unwrap());
    Ok(Kdf { m_kib, t, p })
}

fn decrypt(bytes: &[u8], password: &[u8], kdf: Kdf) -> Result<Zeroizing<[u8; 16]>, Error> {
    let salt = &bytes[17..17 + SALT_LEN];
    let nonce = &bytes[17 + SALT_LEN..17 + SALT_LEN + NONCE_LEN];
    let ct = &bytes[17 + SALT_LEN + NONCE_LEN..];
    let mut key = derive_key(password, salt, kdf)?;
    let cipher = ChaCha20Poly1305::new_from_slice(key.as_slice())
        .map_err(|_| Error::Vault("无法打开保险库"))?;
    let plain = cipher.decrypt(Nonce::from_slice(nonce), ct);
    key.zeroize();
    let plain = plain.map_err(|_| Error::Vault("无法打开保险库"))?;
    if plain.len() != PLAIN_LEN {
        return Err(Error::Vault("无法打开保险库"));
    }
    let mut entropy = Zeroizing::new([0u8; 16]);
    entropy.copy_from_slice(&plain);
    Ok(entropy)
}

fn derive_key(password: &[u8], salt: &[u8], kdf: Kdf) -> Result<Zeroizing<[u8; 32]>, Error> {
    let params = Params::new(kdf.m_kib, kdf.t, kdf.p, Some(32)).map_err(|_| Error::Vault("保险库口令参数不对"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(password, salt, key.as_mut_slice())
        .map_err(|_| Error::Vault("保险库口令派生失败"))?;
    Ok(key)
}

pub fn random_entropy() -> Result<[u8; 16], Error> {
    let mut entropy = [0u8; 16];
    getrandom::getrandom(&mut entropy).map_err(|_| Error::Vault("无法读取系统随机数"))?;
    Ok(entropy)
}

#[cfg(test)]
fn seal_weak(password: &[u8], entropy: &[u8; 16]) -> Vec<u8> {
    // Argon2id 允许的最小内存，只用于测试错误口令和降级拒绝。
    seal_with(
        password,
        entropy,
        Kdf {
            m_kib: 8,
            t: 1,
            p: 1,
        },
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_password_and_downgrade_and_no_plaintext() {
        let entropy = [0x11u8; 16];
        let file = seal_weak(b"correct-horse", &entropy);
        assert!(!file.windows(16).any(|w| w == entropy));
        let opened = decrypt(
            &file,
            b"correct-horse",
            Kdf {
                m_kib: 8,
                t: 1,
                p: 1,
            },
        )
        .unwrap();
        assert_eq!(&*opened, &entropy);
        assert_eq!(
            decrypt(
                &file,
                b"wrong-password",
                Kdf {
                    m_kib: 8,
                    t: 1,
                    p: 1,
                },
            )
            .unwrap_err()
            .to_string(),
            "无法打开保险库"
        );
        assert_eq!(
            open(&file, b"correct-horse").unwrap_err().to_string(),
            "保险库口令参数不对"
        );

        let mut tampered = file.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(decrypt(
            &tampered,
            b"correct-horse",
            Kdf {
                m_kib: 8,
                t: 1,
                p: 1,
            },
        )
        .is_err());

        let mut extra = file.clone();
        extra.push(0);
        assert!(open(&extra, b"correct-horse").is_err());
    }
}
