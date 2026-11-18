//! 预览区（Strip）窗口。
//!
//! 常驻屏幕左侧的窄栏，展示非活跃窗口的 DWM 缩略图。窗口使用
//! `WS_EX_NOACTIVATE`，点击不会抢走用户正在输入的窗口的焦点。

pub mod window;

pub use window::Strip;
