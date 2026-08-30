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

/// 시작 시 Run 키를 현재 실행 파일 경로로 맞춘다.
///
/// `set`은 토글을 켠 순간의 경로를 문자열로 박아두기 때문에, 포터블 exe로 한 번 켜두면
/// 이후 설치본을 깔아도 부팅 시에는 계속 옛 경로가 실행된다(재설치로는 고쳐지지 않음).
/// 마지막으로 실행한 exe가 자동시작 대상이 되도록 매 기동 시 경로를 갱신한다.
#[cfg(windows)]
pub fn sync(enabled: bool) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = match hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_SET_VALUE) {
        Ok(k) => k,
        Err(_) => return,
    };
    let current = key.get_value::<String, _>(VALUE_NAME).ok();

    if !enabled {
        if current.is_some() {
            let _ = key.delete_value(VALUE_NAME);
        }
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let desired = format!("\"{}\"", exe.display());
    if current.as_deref() != Some(desired.as_str()) {
        let _ = key.set_value(VALUE_NAME, &desired);
    }
}

#[cfg(not(windows))]
pub fn sync(_enabled: bool) {}
