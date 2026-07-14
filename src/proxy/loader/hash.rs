use std::fs;

use windows_sys_loader::Win32::Security::Cryptography::{
    BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash,
    BCryptGetProperty, BCryptHashData, BCryptOpenAlgorithmProvider, BCRYPT_ALG_HANDLE,
    BCRYPT_HASH_HANDLE, BCRYPT_OBJECT_LENGTH, BCRYPT_SHA256_ALGORITHM,
};

/// 计算文件 SHA256，返回大写十六进制字符串（构建指纹，验证版本一致性）
pub fn sha256_file(path: &str) -> Option<String> {
    let data = fs::read(path).ok()?;
    let digest = sha256(&data)?;
    Some(digest.iter().map(|b| format!("{:02X}", b)).collect())
}

unsafe fn open_algo() -> Option<BCRYPT_ALG_HANDLE> {
    let mut algo: BCRYPT_ALG_HANDLE = std::ptr::null_mut();
    let status =
        BCryptOpenAlgorithmProvider(&mut algo, BCRYPT_SHA256_ALGORITHM, std::ptr::null(), 0);
    if status != 0 {
        return None;
    }
    Some(algo)
}

fn sha256(data: &[u8]) -> Option<[u8; 32]> {
    unsafe {
        let algo = open_algo()?;

        let mut obj_len: u32 = 0;
        let mut result_len: u32 = 0;
        BCryptGetProperty(
            algo,
            BCRYPT_OBJECT_LENGTH,
            &mut obj_len as *mut u32 as *mut u8,
            std::mem::size_of::<u32>() as u32,
            &mut result_len,
            0,
        );

        let mut obj_buf = vec![0u8; obj_len as usize];
        let mut hash: BCRYPT_HASH_HANDLE = std::ptr::null_mut();
        let status = BCryptCreateHash(
            algo,
            &mut hash,
            obj_buf.as_mut_ptr(),
            obj_len,
            std::ptr::null(),
            0,
            0,
        );
        if status != 0 {
            BCryptCloseAlgorithmProvider(algo, 0);
            return None;
        }

        BCryptHashData(hash, data.as_ptr(), data.len() as u32, 0);

        let mut digest = [0u8; 32];
        BCryptFinishHash(hash, digest.as_mut_ptr(), digest.len() as u32, 0);

        BCryptDestroyHash(hash);
        BCryptCloseAlgorithmProvider(algo, 0);
        Some(digest)
    }
}
