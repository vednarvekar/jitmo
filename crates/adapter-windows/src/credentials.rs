//! The database key, kept in Windows Credential Manager.
//!
//! One generic credential (target `jitmo/db-key`, persisted for this machine
//! and user) holds 32 random bytes as 64 hex characters. If it is missing it
//! is created from the OS random number generator. The app passes the result
//! to `jitmo_store::Store::open(path, Some(&key))`:
//!
//! ```ignore
//! let key = jitmo_adapter_windows::credentials::get_or_create_db_key("jitmo")?;
//! let store = Store::open(&db_path, Some(&key))?;
//! ```
//!
//! The hex helpers are portable and tested everywhere; the Credential Manager
//! calls exist only on Windows.

/// Credential Manager target name for the database key.
pub const DB_KEY_TARGET: &str = "jitmo/db-key";

/// 32 bytes as 64 lowercase hex characters.
pub fn encode_key(key: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(64);
    for b in key {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Inverse of `encode_key`. `None` unless the input is exactly 64 hex characters.
pub fn decode_key(blob: &[u8]) -> Option<[u8; 32]> {
    if blob.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in blob.chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

#[cfg(windows)]
mod os {
    use std::ptr;

    const CRED_TYPE_GENERIC: u32 = 1;
    const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
    const ERROR_NOT_FOUND: i32 = 1168;
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;

    #[repr(C)]
    struct CredentialW {
        flags: u32,
        kind: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: [u32; 2],
        blob_size: u32,
        blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut u8,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn CredReadW(target: *const u16, kind: u32, flags: u32, out: *mut *mut CredentialW) -> i32;
        fn CredWriteW(cred: *const CredentialW, flags: u32) -> i32;
        fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
        fn CredFree(buffer: *mut core::ffi::c_void);
    }

    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(alg: *mut core::ffi::c_void, buf: *mut u8, len: u32, flags: u32) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn last_error() -> i32 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

    pub fn random_key() -> Result<[u8; 32], String> {
        random32()
    }

    pub fn random32() -> Result<[u8; 32], String> {
        let mut key = [0u8; 32];
        // SAFETY: `key` is valid for 32 bytes; a null algorithm handle with the
        // system-preferred-RNG flag is the documented way to call this.
        let status = unsafe {
            BCryptGenRandom(
                ptr::null_mut(),
                key.as_mut_ptr(),
                32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status != 0 {
            return Err(format!("BCryptGenRandom failed: 0x{status:08x}"));
        }
        Ok(key)
    }

    /// `Ok(None)` if the credential does not exist.
    pub fn read(target: &str) -> Result<Option<Vec<u8>>, String> {
        let t = wide(target);
        let mut cred: *mut CredentialW = ptr::null_mut();
        // SAFETY: `t` is NUL-terminated and outlives the call; `cred` receives
        // a buffer that we free with CredFree below.
        let ok = unsafe { CredReadW(t.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) };
        if ok == 0 {
            let e = last_error();
            return if e == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(format!("CredReadW failed: error {e}"))
            };
        }
        // SAFETY: CredReadW succeeded, so `cred` points at a valid CREDENTIALW
        // whose blob is `blob_size` bytes; we copy it out before freeing.
        let bytes = unsafe {
            let c = &*cred;
            let v = if c.blob.is_null() || c.blob_size == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(c.blob, c.blob_size as usize).to_vec()
            };
            CredFree(cred.cast());
            v
        };
        Ok(Some(bytes))
    }

    pub fn write(target: &str, user: &str, blob: &[u8]) -> Result<(), String> {
        let t = wide(target);
        let u = wide(user);
        let cred = CredentialW {
            flags: 0,
            kind: CRED_TYPE_GENERIC,
            target_name: t.as_ptr().cast_mut(),
            comment: ptr::null_mut(),
            last_written: [0, 0],
            blob_size: blob.len() as u32,
            blob: blob.as_ptr().cast_mut(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            attribute_count: 0,
            attributes: ptr::null_mut(),
            target_alias: ptr::null_mut(),
            user_name: u.as_ptr().cast_mut(),
        };
        // SAFETY: all pointers in `cred` reference buffers that live past the
        // call; CredWriteW only reads them.
        let ok = unsafe { CredWriteW(&cred, 0) };
        if ok == 0 {
            return Err(format!("CredWriteW failed: error {}", last_error()));
        }
        Ok(())
    }

    pub fn delete(target: &str) -> Result<(), String> {
        let t = wide(target);
        // SAFETY: `t` is NUL-terminated and outlives the call.
        let ok = unsafe { CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if ok == 0 && last_error() != ERROR_NOT_FOUND {
            return Err(format!("CredDeleteW failed: error {}", last_error()));
        }
        Ok(())
    }
}

#[cfg(windows)]
fn get_or_create(target: &str, user: &str) -> Result<[u8; 32], String> {
    if let Some(blob) = os::read(target)? {
        // A stored value of the wrong shape is an error, never silently replaced:
        // replacing it would lock the user out of an existing encrypted database.
        return decode_key(&blob)
            .ok_or_else(|| format!("credential {target} exists but is not a 32-byte hex key"));
    }
    let key = os::random_key()?;
    os::write(target, user, encode_key(&key).as_bytes())?;
    Ok(key)
}

/// A secret text kept in Credential Manager under `target` (for example the Microsoft
/// refresh token, `jitmo/ms365/token`). `Ok(None)` when there is none.
#[cfg(windows)]
pub fn read_secret(target: &str) -> Result<Option<String>, String> {
    Ok(os::read(target)?.map(|b| String::from_utf8_lossy(&b).into_owned()))
}

/// Store or replace a secret text.
#[cfg(windows)]
pub fn write_secret(target: &str, user: &str, value: &str) -> Result<(), String> {
    os::write(target, user, value.as_bytes())
}

/// Remove a secret (signing out). Removing one that is not there is not an error.
#[cfg(windows)]
pub fn delete_secret(target: &str) -> Result<(), String> {
    os::delete(target)
}

/// 32 random bytes from the operating system.
#[cfg(windows)]
pub fn random_bytes32() -> Result<[u8; 32], String> {
    os::random32()
}

/// Read the database key from Credential Manager, creating it on first use.
/// `service` is stored as the credential's user name (for display only).
#[cfg(windows)]
pub fn get_or_create_db_key(service: &str) -> Result<[u8; 32], String> {
    get_or_create(DB_KEY_TARGET, service)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(37).wrapping_add(11);
        }
        let s = encode_key(&key);
        assert_eq!(s.len(), 64);
        assert_eq!(decode_key(s.as_bytes()), Some(key));
        assert_eq!(decode_key(s.to_uppercase().as_bytes()), Some(key));
    }

    #[test]
    fn decode_rejects_bad_input() {
        assert_eq!(decode_key(b""), None);
        assert_eq!(decode_key(&[b'a'; 63]), None);
        assert_eq!(decode_key(&[b'g'; 64]), None);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "touches the real Windows Credential Manager"]
    fn live_create_then_read_back() {
        let target = format!("jitmo-test/db-key-{}", std::process::id());
        let first = get_or_create(&target, "jitmo-test").unwrap();
        let second = get_or_create(&target, "jitmo-test").unwrap();
        os::delete(&target).unwrap();
        assert_eq!(first, second);
        assert_ne!(first, [0u8; 32]);
        assert!(os::read(&target).unwrap().is_none());
    }
}
