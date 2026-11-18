//! AirPane 主程序入口。
//!
//! M1 阶段：加载配置、初始化日志。Stage 布局、模式切换与预览区在后续 PR 接入。

use airpane_core::config::CONFIG_VERSION;
use airpane_platform_win::{config_io, log_info, log_warn, logging, paths};

fn main() {
    if let Err(err) = paths::ensure_app_dir() {
        eprintln!("无法创建应用数据目录: {err}");
    }

    let config_path = paths::config_file();
    let (config, notice) = config_io::load_or_create(&config_path);

    let log_path = paths::log_file();
    let log_ready = match logging::init(config.log.level, &log_path) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("日志初始化失败: {err}");
            false
        }
    };

    if let Some(notice) = notice {
        log_warn!("{notice}");
        eprintln!("{notice}");
    }
    if log_ready {
        log_info!(
            "AirPane {} 启动：配置格式 v{}，日志级别 {:?}",
            env!("CARGO_PKG_VERSION"),
            CONFIG_VERSION,
            config.log.level
        );
    }

    println!(
        "AirPane {} — M1 自用版（配置格式 v{}）",
        env!("CARGO_PKG_VERSION"),
        CONFIG_VERSION
    );
    println!("配置：{}", config_path.display());
    println!("日志：{}", log_path.display());
    println!("预览区宽度：{} DIP（设定值）", config.strip.width_dip);
    println!("模式切换快捷键：{}", config.hotkey.toggle_mode);
    println!("启动模式：{:?}", config.startup.mode);
}
