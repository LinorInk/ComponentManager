//! 用户级 PATH 管理（仅 Windows）：把 cmcli 所在目录加入 / 移出用户环境变量。

use std::env;
use std::path::PathBuf;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// cmcli.exe 所在目录
fn self_dir() -> Result<PathBuf, String> {
    let exe = env::current_exe().map_err(err)?;
    exe.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法确定程序所在目录".to_string())
}

#[cfg(windows)]
fn user_path() -> Result<String, String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;
    let env_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ)
        .map_err(err)?;
    Ok(match env_key.get_raw_value("Path") {
        Ok(v) => {
            let s = String::from_utf16_lossy(utf16_bytes_to_u16(&v.bytes));
            s.trim_end_matches('\0').to_string()
        }
        Err(_) => String::new(),
    })
}

#[cfg(windows)]
fn utf16_bytes_to_u16(bytes: &[u8]) -> &[u16] {
    // 注册表 REG_*_SZ 为偶数字节 UTF-16LE，此转换安全
    unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u16, bytes.len() / 2) }
}

#[cfg(windows)]
fn write_user_path(value: &str) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE, REG_EXPAND_SZ};
    use winreg::RegKey;
    let env_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_WRITE)
        .map_err(err)?;
    env_key
        .set_raw_value(
            "Path",
            &winreg::RegValue {
                bytes: value
                    .encode_utf16()
                    .flat_map(|c| c.to_ne_bytes())
                    .chain([0, 0, 0, 0])
                    .collect(),
                vtype: REG_EXPAND_SZ,
            },
        )
        .map_err(err)
}

/// 广播 WM_SETTINGCHANGE，让资源管理器刷新用户环境变量：
/// 新开的终端立即能用上新的 PATH，无需注销或重启 Explorer
#[cfg(windows)]
fn broadcast_env_change() {
    use std::ffi::c_void;
    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: *mut c_void,
            msg: u32,
            wparam: usize,
            lparam: *const u16,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const HWND_BROADCAST: usize = 0xffff;
    let target: Vec<u16> = "Environment".encode_utf16().chain([0u16]).collect();
    let mut res: usize = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST as *mut c_void,
            WM_SETTINGCHANGE,
            0,
            target.as_ptr(),
            SMTO_ABORTIFHUNG,
            5000,
            &mut res,
        );
    }
}

/// 把 cmcli 所在目录加入用户 PATH（幂等）。返回 (状态, 目录)。
#[cfg(windows)]
pub fn add() -> Result<(&'static str, String), String> {
    let dir_s = self_dir()?.to_string_lossy().to_string();
    let cur = user_path()?;
    if is_present(&cur, &dir_s) {
        return Ok(("already", dir_s));
    }
    let new_val = if cur.is_empty() {
        dir_s.clone()
    } else {
        format!("{};{}", cur.trim_end_matches(';'), dir_s)
    };
    write_user_path(&new_val)?;
    broadcast_env_change();
    Ok(("added", dir_s))
}

/// 从用户 PATH 移除 cmcli 所在目录，不动其他条目。返回 (状态, 目录)。
#[cfg(windows)]
pub fn remove() -> Result<(&'static str, String), String> {
    let dir_s = self_dir()?.to_string_lossy().to_string();
    let cur = user_path()?;
    if !is_present(&cur, &dir_s) {
        return Ok(("not-in-path", dir_s));
    }
    let kept: Vec<&str> = cur
        .split(';')
        .filter(|x| !x.trim_end_matches('\\').eq_ignore_ascii_case(&dir_s))
        .collect();
    write_user_path(&kept.join(";"))?;
    broadcast_env_change();
    Ok(("removed", dir_s))
}

/// PATH 条目比较：忽略大小写与结尾反斜杠
#[cfg(windows)]
fn is_present(cur: &str, dir: &str) -> bool {
    cur.split(';')
        .any(|x| x.trim_end_matches('\\').eq_ignore_ascii_case(dir))
}

#[cfg(not(windows))]
pub fn add() -> Result<(&'static str, String), String> {
    Err("仅 Windows 支持 PATH 管理".into())
}

#[cfg(not(windows))]
pub fn remove() -> Result<(&'static str, String), String> {
    Err("仅 Windows 支持 PATH 管理".into())
}
