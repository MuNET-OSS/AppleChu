//! RSA 验签与 base64 解码
//!
//! 服务器下发的资源包由 MuNET 私钥签名，客户端用内置公钥验签。
//! 签名算法与 AquaMai 一致：RSA PKCS#1 v1.5 + SHA-256。
//! 公钥以 base64 编码的 modulus 形式给出，指数固定 65537。

use std::ffi::c_void;
use std::ptr;

use windows_sys::Win32::Security::Cryptography::{
    BCryptCloseAlgorithmProvider, BCryptDestroyKey, BCryptImportKeyPair, BCryptOpenAlgorithmProvider,
    BCryptVerifySignature, BCRYPT_ALG_HANDLE, BCRYPT_KEY_HANDLE, BCRYPT_PAD_PKCS1,
    BCRYPT_PKCS1_PADDING_INFO, BCRYPT_RSA_ALGORITHM, BCRYPT_RSAPUBLIC_BLOB,
    BCRYPT_SHA256_ALGORITHM,
};

use crate::util::hash::sha256;

/// `BCRYPT_RSAKEY_BLOB` 的公钥魔数（"RSA1"）
const BCRYPT_RSAPUBLIC_MAGIC: u32 = 0x3141_5352;
/// 固定公钥指数 65537（大端字节序）
const PUBLIC_EXPONENT: [u8; 3] = [0x01, 0x00, 0x01];

/// 解码标准 base64（忽略空白，要求正确的 `=` 填充）
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    let mut accumulator: u32 = 0;
    let mut bits = 0u32;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
        }
    }

    Some(output)
}

/// 用内置公钥验证 `data` 上的 `signature`（base64）
///
/// `modulus_base64` 是 RSA 公钥的 base64 modulus；解析或验证失败均返回 `false`。
pub fn verify_signature(modulus_base64: &str, data: &[u8], signature_base64: &str) -> bool {
    let Some(modulus) = base64_decode(modulus_base64) else {
        return false;
    };
    let Some(signature) = base64_decode(signature_base64) else {
        return false;
    };
    let Some(digest) = sha256(data) else {
        return false;
    };
    verify_rsa_sha256(&modulus, &signature, &digest)
}

/// 组装 `BCRYPT_RSAPUBLIC_BLOB` 并调用 BCrypt 验签
fn verify_rsa_sha256(modulus: &[u8], signature: &[u8], digest: &[u8; 32]) -> bool {
    if modulus.is_empty() || signature.is_empty() {
        return false;
    }
    let Ok(modulus_len) = u32::try_from(modulus.len()) else {
        return false;
    };

    // BCRYPT_RSAKEY_BLOB 头部 + 公钥指数 + modulus
    let mut blob = Vec::with_capacity(24 + PUBLIC_EXPONENT.len() + modulus.len());
    blob.extend_from_slice(&BCRYPT_RSAPUBLIC_MAGIC.to_le_bytes());
    blob.extend_from_slice(&(modulus.len() as u32 * 8).to_le_bytes()); // BitLength
    blob.extend_from_slice(&(PUBLIC_EXPONENT.len() as u32).to_le_bytes()); // cbPublicExp
    blob.extend_from_slice(&modulus_len.to_le_bytes()); // cbModulus
    blob.extend_from_slice(&0u32.to_le_bytes()); // cbPrime1
    blob.extend_from_slice(&0u32.to_le_bytes()); // cbPrime2
    blob.extend_from_slice(&PUBLIC_EXPONENT);
    blob.extend_from_slice(modulus);

    unsafe {
        let mut algorithm: BCRYPT_ALG_HANDLE = ptr::null_mut();
        if BCryptOpenAlgorithmProvider(&mut algorithm, BCRYPT_RSA_ALGORITHM, ptr::null(), 0) != 0 {
            return false;
        }

        let mut key: BCRYPT_KEY_HANDLE = ptr::null_mut();
        let imported = BCryptImportKeyPair(
            algorithm,
            ptr::null_mut(),
            BCRYPT_RSAPUBLIC_BLOB,
            &mut key,
            blob.as_ptr(),
            blob.len() as u32,
            0,
        ) == 0;

        let verified = if imported {
            let padding = BCRYPT_PKCS1_PADDING_INFO {
                pszAlgId: BCRYPT_SHA256_ALGORITHM,
            };
            BCryptVerifySignature(
                key,
                (&padding as *const BCRYPT_PKCS1_PADDING_INFO).cast::<c_void>(),
                digest.as_ptr(),
                digest.len() as u32,
                signature.as_ptr(),
                signature.len() as u32,
                BCRYPT_PAD_PKCS1,
            ) == 0
        } else {
            false
        };

        if !key.is_null() {
            BCryptDestroyKey(key);
        }
        BCryptCloseAlgorithmProvider(algorithm, 0);
        verified
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_known_vectors() {
        // Given: RFC 4648 的标准测试向量。
        let cases = [
            ("", ""),
            ("Zg==", "f"),
            ("Zm8=", "fo"),
            ("Zm9v", "foo"),
            ("Zm9vYmFy", "foobar"),
            ("+/8=", "\u{ff}\u{fe}"),
        ];

        // When/Then: 解码结果与原文一致。
        for (encoded, plain) in cases {
            assert_eq!(base64_decode(encoded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn base64_rejects_invalid_alphabet() {
        // Given: 含非法字符的输入。
        // Then: 解码失败而不是静默给出错误字节。
        assert!(base64_decode("Zm9v*").is_none());
    }

    #[test]
    fn empty_modulus_never_verifies() {
        // Given: 未配置公钥（空 modulus）。
        // Then: 任何签名都不通过，避免"无公钥即放行"。
        assert!(!verify_signature("", b"data", "AAAA"));
    }
}
