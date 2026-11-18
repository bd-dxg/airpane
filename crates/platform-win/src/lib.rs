//! Win32 封装层。
//!
//! 项目里所有 `unsafe` 代码都集中在 `airpane-platform-win` 与 `airpane-strip`，
//! 每处 `unsafe` 块前必须写明安全前提。

pub mod config_io;
pub mod dialog;
pub mod events;
pub mod foreground;
pub mod hotkey;
pub mod icon;
pub mod json_io;
pub mod logging;
pub mod message_loop;
pub mod monitor;
pub mod paths;
pub mod placement;
pub mod size_store;
pub mod snapshot_store;
pub mod strip_order_store;
pub mod window;
