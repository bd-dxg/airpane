//! AirPane 主程序入口。
//!
//! M0 阶段只验证核心能力（窗口枚举、事件同步、预览区、前台切换、动画路线），
//! 入口暂不承载托盘、热键与配置。

fn main() {
    println!(
        "AirPane {} — M0 原型阶段（快照格式 v{}）",
        env!("CARGO_PKG_VERSION"),
        airpane_core::snapshot::SNAPSHOT_VERSION
    );
}
