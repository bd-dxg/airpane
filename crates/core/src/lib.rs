//! AirPane 核心模型。
//!
//! 本 crate 不依赖 Win32，只放可在任意平台上单元测试的纯逻辑：窗口状态、
//! 快照结构、分组、布局与宽度钳制计算。

pub mod snapshot;
