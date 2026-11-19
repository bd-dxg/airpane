//! 系统应用主题（浅色 / 深色）。预览区按它选配色（D-054）。

use core::ffi::c_void;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_DWORD};
use windows::Win32::UI::Shell::SHGetValueW;
use windows::core::{PCWSTR, w};

/// 系统应用主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    Light,
    /// 读不到设置时的兜底：保持 M1 原有的深色配色。
    #[default]
    Dark,
}

const PERSONALIZE_KEY: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
const APPS_USE_LIGHT_THEME: PCWSTR = w!("AppsUseLightTheme");

/// 读注册表判断系统「应用模式」是浅色还是深色。
///
/// 键不存在（老系统、精简系统）时按深色处理，不影响其他功能。
pub fn system_color_scheme() -> ColorScheme {
    let mut value: u32 = 0;
    let mut value_type = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: 输出缓冲是本栈帧上的 `u32`，长度如实传入；只读查询，句柄是系统预定义的。
    let status = unsafe {
        SHGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE_KEY,
            APPS_USE_LIGHT_THEME,
            Some(&mut value_type),
            Some(&mut value as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    // 只认 DWORD：值是其他类型时这 4 个字节是垃圾数据，不能拿来判断。
    match status {
        ERROR_SUCCESS if value_type == REG_DWORD.0 && value != 0 => ColorScheme::Light,
        _ => ColorScheme::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_read_does_not_panic() {
        // 开发机一定读得到；CI runner 读不到也只会回退成深色。
        let scheme = system_color_scheme();
        assert!(matches!(scheme, ColorScheme::Light | ColorScheme::Dark));
    }
}
