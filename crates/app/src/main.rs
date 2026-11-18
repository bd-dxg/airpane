//! AirPane 主程序入口。
//!
//! M1 阶段：加载配置与日志、处理崩溃残留的快照、在主线程跑预览区与 Stage 布局。
//! 模式切换快捷键与底部工具区在下一个 PR 接入。

mod app;
mod recovery;

use std::process::ExitCode;

use airpane_core::config::CONFIG_VERSION;
use airpane_platform_win::events::EventMonitor;
use airpane_platform_win::message_loop::{self, LoopMessage};
use airpane_platform_win::{config_io, log_info, log_warn, logging, paths, size_store};
use airpane_strip::window::{Strip, enable_per_monitor_v2};

fn main() -> ExitCode {
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

    // 上次没正常退出时先把窗口摆回原位，再做别的。
    let recovered = recovery::recover_if_needed(&paths::snapshot_file());
    if recovered > 0 {
        println!("崩溃恢复：已把 {recovered} 个窗口摆回快照记录的位置");
    }

    enable_per_monitor_v2();
    let mut strip = match Strip::create(config.strip.width_dip) {
        Ok(strip) => strip,
        Err(err) => {
            log_warn!("创建预览区失败: {err}");
            eprintln!("创建预览区失败: {err}");
            return ExitCode::FAILURE;
        }
    };

    let sizes = size_store::read(&paths::window_sizes_file()).unwrap_or_default();
    let mut app = app::App::new(sizes, paths::snapshot_file(), paths::window_sizes_file());

    let (monitor, events) = match EventMonitor::start() {
        Ok((monitor, events)) => (monitor, events),
        Err(err) => {
            log_warn!("事件监听启动失败: {err}");
            eprintln!("事件监听启动失败: {err}");
            return ExitCode::FAILURE;
        }
    };

    app.enter_stage(&mut strip);
    println!("AirPane {} — 已进入 Stage 模式", env!("CARGO_PKG_VERSION"));
    println!("配置：{}", config_path.display());
    println!("日志：{}", log_path.display());
    println!("按 Ctrl+C 退出；被移动的窗口会在退出时还原");

    message_loop::run(1000, |message| match message {
        LoopMessage::Wakeup => {
            while let Ok(event) = events.try_recv() {
                app.on_event(event, &mut strip);
            }
        }
        LoopMessage::Timer => app.reconcile(&mut strip),
    });

    drop(monitor);
    // `app` 在这里析构，Drop 会把移动过的窗口还原并清掉快照。
    drop(app);
    ExitCode::SUCCESS
}
