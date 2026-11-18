//! Win32 封装层。
//!
//! 项目里所有 `unsafe` 代码都集中在 `airpane-platform-win` 与 `airpane-strip`，
//! 每处 `unsafe` 块前必须写明安全前提。

pub mod events;
pub mod window;
