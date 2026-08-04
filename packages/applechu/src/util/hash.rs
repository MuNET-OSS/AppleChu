use std::fs;

use windows_sys::Win32::Security::Cryptography::{
    BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash,
    BCryptGetProperty, BCryptHashData, BCryptOpenAlgorithmProvider, BCRYPT_ALG_HANDLE,
    BCRYPT_HASH_HANDLE, BCRYPT_OBJECT_LENGTH, BCRYPT_SHA256_ALGORITHM,
};

pub fn sha256_file(path: impl AsRef<std::path::Path>) -> Option<String> {
    let data = fs::read(path).ok()?;
    let digest = sha256(&data)?;
    Some(digest.iter().map(|byte| format!("{byte:02X}")).collect())
}

fn sha256(data: &[u8]) -> Option<[u8; 32]> {
    unsafe {
        let mut algorithm: BCRYPT_ALG_HANDLE = std::ptr::null_mut();
        if BCryptOpenAlgorithmProvider(&mut algorithm, BCRYPT_SHA256_ALGORITHM, std::ptr::null(), 0)
            != 0
        {
            return None;
        }

        let mut object_length = 0;
        let mut result_length = 0;
        if BCryptGetProperty(
            algorithm,
            BCRYPT_OBJECT_LENGTH,
            (&mut object_length as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
            &mut result_length,
            0,
        ) != 0
        {
            BCryptCloseAlgorithmProvider(algorithm, 0);
            return None;
        }

        let mut object = vec![0u8; object_length as usize];
        let mut hash: BCRYPT_HASH_HANDLE = std::ptr::null_mut();
        if BCryptCreateHash(
            algorithm,
            &mut hash,
            object.as_mut_ptr(),
            object_length,
            std::ptr::null(),
            0,
            0,
        ) != 0
        {
            BCryptCloseAlgorithmProvider(algorithm, 0);
            return None;
        }

        let hash_status = BCryptHashData(hash, data.as_ptr(), data.len() as u32, 0);
        let mut digest = [0u8; 32];
        let finish_status = BCryptFinishHash(hash, digest.as_mut_ptr(), digest.len() as u32, 0);
        BCryptDestroyHash(hash);
        BCryptCloseAlgorithmProvider(algorithm, 0);

        (hash_status == 0 && finish_status == 0).then_some(digest)
    }
}
