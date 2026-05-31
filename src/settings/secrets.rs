use serde::{Deserialize, Deserializer, Serializer};
use std::slice;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

const DPAPI_PREFIX: &str = "dpapi:v1:";

pub(super) fn serialize_secret<S>(value: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if value.is_empty() {
        return serializer.serialize_str("");
    }
    let protected = protect_secret_for_storage(value).map_err(serde::ser::Error::custom)?;
    serializer.serialize_str(&protected)
}

pub(super) fn deserialize_secret<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if let Some(encoded) = value.strip_prefix(DPAPI_PREFIX) {
        return Ok(unprotect_secret_from_storage(encoded).unwrap_or_default());
    }
    Ok(value)
}

pub(super) fn protect_secret_for_storage(value: &str) -> Result<String, String> {
    let protected = dpapi_protect(value.as_bytes())?;
    Ok(format!("{DPAPI_PREFIX}{}", hex_encode(&protected)))
}

fn unprotect_secret_from_storage(encoded: &str) -> Result<String, String> {
    let bytes = hex_decode(encoded)?;
    let plain = dpapi_unprotect(&bytes)?;
    String::from_utf8(plain).map_err(|err| err.to_string())
}

fn dpapi_protect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut input = blob_from_bytes(bytes);
    let mut output = CRYPT_INTEGER_BLOB::default();
    let description = crate::util::wide("claudie profile secret");
    let ok = unsafe {
        CryptProtectData(
            &mut input,
            description.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    bytes_from_owned_blob(output)
}

fn dpapi_unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut input = blob_from_bytes(bytes);
    let mut output = CRYPT_INTEGER_BLOB::default();
    let mut description = std::ptr::null_mut();
    let ok = unsafe {
        CryptUnprotectData(
            &mut input,
            &mut description,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if !description.is_null() {
        unsafe {
            LocalFree(description as _);
        }
    }
    if ok == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    bytes_from_owned_blob(output)
}

fn blob_from_bytes(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: bytes.len().min(u32::MAX as usize) as u32,
        pbData: bytes.as_ptr() as *mut u8,
    }
}

fn bytes_from_owned_blob(blob: CRYPT_INTEGER_BLOB) -> Result<Vec<u8>, String> {
    if blob.pbData.is_null() {
        return Ok(Vec::new());
    }
    let bytes = unsafe { slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec() };
    unsafe {
        LocalFree(blob.pbData as _);
    }
    Ok(bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("encrypted secret has odd hex length".to_string());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("encrypted secret contains non-hex characters".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_secret_round_trips_for_storage() {
        let protected = protect_secret_for_storage("secret-value").unwrap();
        assert!(protected.starts_with(DPAPI_PREFIX));
        let encoded = protected.strip_prefix(DPAPI_PREFIX).unwrap();
        assert_eq!(
            unprotect_secret_from_storage(encoded).unwrap(),
            "secret-value"
        );
    }
}
