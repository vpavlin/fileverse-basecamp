//! The Fileverse primitives, byte-compatible with `@fileverse/crypto` and the
//! ddocs.new app: NaCl secretbox, Argon2id + HKDF for link keys, PBKDF2 + AES-CBC
//! for v3 link locks, AES-256-GCM for content and titles, and secp256k1 ECIES.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use crypto_secretbox::XSalsa20Poly1305;
use hkdf::Hkdf;
use sha2::Sha256;

use crate::b64;

pub const SEP: &str = "__n__";

pub fn secretbox_open(key: &[u8], nonce: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    if key.len() != 32 || nonce.len() != 24 {
        return None;
    }
    let c = XSalsa20Poly1305::new_from_slice(key).ok()?;
    c.decrypt(crypto_secretbox::Nonce::from_slice(nonce), ct).ok()
}

/// `nonce__n__ciphertext`, both base64, opened with a 32-byte secretbox key.
pub fn open_nonce_appended(s: &str, key: &[u8]) -> Option<Vec<u8>> {
    let (n, c) = s.split_once(SEP)?;
    secretbox_open(key, &b64::decode(n).ok()?, &b64::decode(c).ok()?)
}

/// Argon2id memory settings seen in the wild for link keys: the web app uses the
/// `@fileverse/crypto` default (100 MiB), the Fileverse API 16 MiB.
pub const LINK_ARGON_MEMORY_KIB: &[u32] = &[102_400, 16_384];

/// The key that wraps a share link's secret: HKDF(Argon2id(ddocId, nonce)).
pub fn link_wrapping_key(ddoc_id: &str, nonce: &[u8], m_kib: u32) -> Result<[u8; 32], String> {
    let params = argon2::Params::new(m_kib, 2, 8, Some(32)).map_err(|e| e.to_string())?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut hash = [0u8; 32];
    a.hash_password_into(ddoc_id.as_bytes(), nonce, &mut hash)
        .map_err(|e| e.to_string())?;
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(None, &hash)
        .expand(b"encryptionKey", &mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn gcm_open(key: &[u8], iv: &[u8], ct_and_tag: &[u8]) -> Result<Vec<u8>, String> {
    if key.len() != 32 {
        return Err(format!("AES-256 key must be 32 bytes, got {}", key.len()));
    }
    if iv.len() != 12 {
        return Err(format!("GCM nonce must be 12 bytes, got {}", iv.len()));
    }
    Aes256Gcm::new_from_slice(key)
        .map_err(|e| e.to_string())?
        .decrypt(Nonce::from_slice(iv), ct_and_tag)
        .map_err(|_| "AES-GCM authentication failed (wrong key or corrupt data)".to_string())
}

/// `@fileverse/crypto/webcrypto` aesEncrypt layout: 12-byte nonce, then ciphertext+tag.
pub fn gcm_open_nonce_prefixed(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 28 {
        return Err("ciphertext too short".into());
    }
    gcm_open(key, &data[..12], &data[12..])
}

/// The v3 content layout: ciphertext, 16-byte tag, 12-byte IV.
pub fn gcm_open_tag_iv_suffixed(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 28 {
        return Err("encrypted file too small".into());
    }
    let (body, iv) = data.split_at(data.len() - 12);
    gcm_open(key, iv, body)
}

pub fn pbkdf2_32(ikm: &[u8], salt: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(ikm, salt, 32, &mut out);
    out
}

pub fn cbc_open(key: &[u8], iv: &[u8], ct: &[u8]) -> Result<Vec<u8>, String> {
    type Dec = cbc::Decryptor<aes::Aes256>;
    Dec::new_from_slices(key, iv)
        .map_err(|e| e.to_string())?
        .decrypt_padded_vec_mut::<Pkcs7>(ct)
        .map_err(|_| "AES-CBC decryption failed".to_string())
}

/// `@fileverse/crypto/ecies`: `ephemeralPub__n__nonce__n__ciphertext__n__mac`,
/// ECDH on secp256k1 (compressed shared point) -> HKDF-SHA256(salt = ephemeral
/// public key, info "ECIES-AES256-GCM-SHA256") -> AES-256-GCM.
pub fn ecies_open(private_key: &[u8], s: &str) -> Result<Vec<u8>, String> {
    use k256::elliptic_curve::sec1::ToEncodedPoint;
    let parts: Vec<&str> = s.split(SEP).collect();
    if parts.len() != 4 {
        return Err("not an ECIES ciphertext".into());
    }
    let eph = b64::decode(parts[0])?;
    let nonce = b64::decode(parts[1])?;
    let mut ct = b64::decode(parts[2])?;
    ct.extend(b64::decode(parts[3])?);
    let sk = k256::SecretKey::from_slice(private_key).map_err(|_| "bad secp256k1 private key")?;
    let pk = k256::PublicKey::from_sec1_bytes(&eph).map_err(|_| "bad ephemeral public key")?;
    let shared = (pk.to_projective() * *sk.to_nonzero_scalar()).to_affine();
    let shared = shared.to_encoded_point(true);
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&eph), shared.as_bytes())
        .expand(b"ECIES-AES256-GCM-SHA256", &mut key)
        .map_err(|e| e.to_string())?;
    gcm_open(&key, &nonce, &ct)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secretbox_roundtrip() {
        use crypto_secretbox::aead::Aead;
        let key = [7u8; 32];
        let nonce = [9u8; 24];
        let c = XSalsa20Poly1305::new_from_slice(&key).unwrap();
        let ct = c.encrypt(crypto_secretbox::Nonce::from_slice(&nonce), &b"hi"[..]).unwrap();
        assert_eq!(secretbox_open(&key, &nonce, &ct).unwrap(), b"hi");
    }
}
