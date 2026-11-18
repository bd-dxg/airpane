//! 全局快捷键注册。
//!
//! 只用 `RegisterHotKey`（PRD 11.3 第 2 条：不用低级键盘钩子、不记录按键）。注册在
//! 调用线程上，`WM_HOTKEY` 进入该线程的消息队列，由 `message_loop` 转成回调。

use core::fmt;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};

/// 解析后的快捷键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeySpec {
    pub modifiers: HOT_KEY_MODIFIERS,
    /// 虚拟键码。
    pub key: u32,
}

/// 快捷键注册失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    /// 字符串写法不认识。
    Invalid(String),
    /// 系统拒绝注册，通常是已被其他程序占用。
    Rejected(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "快捷键写法无法识别: {reason}"),
            Self::Rejected(reason) => write!(f, "注册快捷键失败: {reason}"),
        }
    }
}

/// 解析 `Ctrl+Alt+S` 这类写法。支持 `Ctrl`/`Alt`/`Shift`/`Win` 与 A–Z、0–9、F1–F12。
pub fn parse(spec: &str) -> Result<HotkeySpec, HotkeyError> {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    let mut key: Option<u32> = None;

    for part in spec.split('+') {
        let part = part.trim();
        if part.is_empty() {
            return Err(HotkeyError::Invalid(format!("{spec} 里有空的键名")));
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "alt" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "super" => modifiers |= MOD_WIN,
            other => {
                if key.is_some() {
                    return Err(HotkeyError::Invalid(format!(
                        "{spec} 里有多个主键，只支持一个"
                    )));
                }
                key = Some(parse_key(other)?);
            }
        }
    }

    let key = key.ok_or_else(|| HotkeyError::Invalid(format!("{spec} 缺少主键")))?;
    Ok(HotkeySpec { modifiers, key })
}

fn parse_key(key: &str) -> Result<u32, HotkeyError> {
    let upper = key.to_ascii_uppercase();
    let mut chars = upper.chars();
    if let (Some(single), None) = (chars.next(), chars.next())
        && single.is_ascii_alphanumeric()
    {
        return Ok(single as u32);
    }
    if let Some(digits) = upper.strip_prefix('F')
        && let Ok(number) = digits.parse::<u32>()
        && (1..=12).contains(&number)
    {
        // F1 = 0x70。
        return Ok(0x70 + number - 1);
    }
    Err(HotkeyError::Invalid(format!("不支持的按键: {key}")))
}

/// 已注册的快捷键，析构时自动注销。
pub struct Hotkey {
    id: i32,
}

impl Hotkey {
    /// 在当前线程上注册快捷键。`id` 用于区分多个快捷键。
    pub fn register(spec: &str, id: i32) -> Result<Self, HotkeyError> {
        let parsed = parse(spec)?;
        // SAFETY: 线程级注册（窗口参数传 None），不绑定任何窗口，也不修改外部状态。
        unsafe { RegisterHotKey(None, id, parsed.modifiers, parsed.key) }
            .map_err(|err| HotkeyError::Rejected(format!("{spec}: {}", err.message())))?;
        Ok(Self { id })
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        // SAFETY: 注销自己注册的同 id 快捷键，参数与注册时一致。
        unsafe {
            let _ = UnregisterHotKey(None, self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_shortcut() {
        let spec = parse("Ctrl+Alt+S").expect("应当解析成功");
        assert_eq!(spec.modifiers, HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0));
        assert_eq!(spec.key, 'S' as u32);
    }

    #[test]
    fn parsing_is_case_insensitive() {
        assert_eq!(parse("ctrl+alt+s").ok(), parse("CTRL+ALT+S").ok());
    }

    #[test]
    fn parses_function_keys() {
        assert_eq!(parse("F1").expect("F1").key, 0x70);
        assert_eq!(parse("Shift+F12").expect("F12").key, 0x7B);
    }

    #[test]
    fn rejects_unknown_keys_and_multiple_keys() {
        assert!(parse("Ctrl+Alt+Mouse4").is_err());
        assert!(parse("Ctrl+A+B").is_err());
        assert!(parse("Ctrl+Alt").is_err());
        assert!(parse("Ctrl++").is_err());
        assert!(parse("F13").is_err());
    }

    #[test]
    fn registering_an_invalid_spec_fails() {
        assert!(matches!(
            Hotkey::register("这不是快捷键", 1),
            Err(HotkeyError::Invalid(_))
        ));
    }
}
