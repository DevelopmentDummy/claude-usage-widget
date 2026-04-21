use crate::errors::{AppError, AppResult};

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE_NAME: &str = "ClaudeUsageWidget";

#[cfg(windows)]
pub fn set(enabled: bool) -> AppResult<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
        .map_err(|e| AppError::Other(format!("registry open: {}", e)))?;

    if enabled {
        let exe = std::env::current_exe()
            .map_err(|e| AppError::Other(format!("current_exe: {}", e)))?;
        let exe_str = format!("\"{}\"", exe.display());
        key.set_value(VALUE_NAME, &exe_str)
            .map_err(|e| AppError::Other(format!("registry set: {}", e)))?;
    } else {
        let _ = key.delete_value(VALUE_NAME);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn set(_enabled: bool) -> AppResult<()> {
    Err(AppError::Other("autostart only supported on Windows".into()))
}

#[cfg(windows)]
pub fn is_enabled() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ) {
        Ok(k) => k.get_value::<String, _>(VALUE_NAME).is_ok(),
        Err(_) => false,
    }
}

#[cfg(not(windows))]
pub fn is_enabled() -> bool { false }
