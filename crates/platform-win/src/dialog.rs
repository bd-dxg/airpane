//! 简单的系统对话框。
//!
//! M1 没有设置面板，快捷键冲突之类需要告知用户的情况用一个模态信息框。

use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};
use windows::core::HSTRING;

/// 弹一个信息框，阻塞当前线程直到用户点确定。
pub fn info(title: &str, text: &str) {
    let text = HSTRING::from(text);
    let caption = HSTRING::from(title);
    // SAFETY: 两个字符串的生命周期覆盖整个调用，窗口参数为空表示不绑定父窗口。
    unsafe {
        MessageBoxW(None, &text, &caption, MB_OK | MB_ICONINFORMATION);
    }
}
