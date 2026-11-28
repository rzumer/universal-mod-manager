//! The few Windows registry calls UMM makes: reading a string (where Steam is installed),
//! and writing and deleting the current user's link handlers.

use std::io;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RRF_RT_REG_SZ,
};

/// A root of the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

impl Hive {
    /// `HKCU`, `HKEY_CURRENT_USER`, `HKLM` or `HKEY_LOCAL_MACHINE`, in any case.
    pub fn parse(name: &str) -> Option<Hive> {
        match name.to_ascii_uppercase().as_str() {
            "HKCU" | "HKEY_CURRENT_USER" => Some(Hive::CurrentUser),
            "HKLM" | "HKEY_LOCAL_MACHINE" => Some(Hive::LocalMachine),
            _ => None,
        }
    }

    fn key(self) -> HKEY {
        match self {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn check(status: u32) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

/// A string value (`""`: the key's default value); `None` when the key or value is missing or
/// not a string.
pub fn read_string(hive: Hive, key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut buf: Vec<u16> = vec![0; 260];
    loop {
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: the strings are NUL-terminated; `bytes` is the buffer's size in bytes.
        let status = unsafe {
            RegGetValueW(
                hive.key(),
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status == ERROR_MORE_DATA {
            buf.resize((bytes as usize).div_ceil(2), 0);
            continue;
        }
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = (bytes as usize / 2).min(buf.len());
        let s = &buf[..len];
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        return Some(String::from_utf16_lossy(&s[..end]));
    }
}

/// Sets a string value (`""`: the key's default value), creating the key as needed.
pub fn write_string(hive: Hive, key: &str, value: &str, data: &str) -> io::Result<()> {
    let key = wide(key);
    let mut handle: HKEY = null_mut();
    // SAFETY: `key` is NUL-terminated; `handle` receives the opened key, closed below.
    check(unsafe {
        RegCreateKeyExW(
            hive.key(),
            key.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &mut handle,
            null_mut(),
        )
    })?;
    let (value, data) = (wide(value), wide(data));
    // SAFETY: `handle` is open; `data` is NUL-terminated and its size given in bytes.
    let set = check(unsafe {
        RegSetValueExW(
            handle,
            value.as_ptr(),
            0,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    });
    // SAFETY: closing the key opened above.
    unsafe { RegCloseKey(handle) };
    set
}

/// Deletes a key with everything under it. A missing key is an error of kind `NotFound`.
pub fn delete_tree(hive: Hive, key: &str) -> io::Result<()> {
    let key = wide(key);
    // SAFETY: `key` is NUL-terminated.
    check(unsafe { RegDeleteTreeW(hive.key(), key.as_ptr()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_missing_value_as_none() {
        assert_eq!(
            read_string(Hive::CurrentUser, r"Software\UMM-no-such-key", "x"),
            None
        );
        assert_eq!(Hive::parse("hkey_local_machine"), Some(Hive::LocalMachine));
        assert_eq!(Hive::parse("HKCR"), None);
    }

    /// Writes a key of its own under the current user's `Software`, and deletes it.
    #[test]
    #[ignore = "writes to the registry: run with --ignored"]
    fn writes_reads_and_deletes_under_the_current_user() {
        let key = format!(r"Software\UMM-test-{}", std::process::id());
        assert_eq!(read_string(Hive::CurrentUser, &key, ""), None);
        let long = "x".repeat(1000);
        write_string(Hive::CurrentUser, &format!(r"{key}\sub"), "", "dé").unwrap();
        write_string(Hive::CurrentUser, &key, "Long", &long).unwrap();
        assert_eq!(
            read_string(Hive::CurrentUser, &format!(r"{key}\sub"), "").as_deref(),
            Some("dé")
        );
        assert_eq!(read_string(Hive::CurrentUser, &key, "Long"), Some(long));
        delete_tree(Hive::CurrentUser, &key).unwrap();
        assert_eq!(read_string(Hive::CurrentUser, &key, "Long"), None);
        assert_eq!(
            delete_tree(Hive::CurrentUser, &key).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
