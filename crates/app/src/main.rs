//! AirPane 主程序入口。
//!
//! M1：加载配置与日志、处理崩溃残留的快照、在主线程跑预览区、Stage 布局与模式切换。
//!
//! release 构建不申请控制台窗口：作为常驻后台程序，弹一个终端只会在提权启动时
//! 多出一个没人看的黑窗。调试信息仍然写进日志文件。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod recovery;

use std::process::ExitCode;
use std::sync::mpsc::channel;

use airpane_core::config::{CONFIG_VERSION, StartupMode};
use airpane_platform_win::events::EventMonitor;
use airpane_platform_win::hotkey::Hotkey;
use airpane_platform_win::message_loop::{self, LoopMessage};
use airpane_platform_win::window::find_window_by_class;
use airpane_platform_win::{
    config_io, dialog, log_info, log_warn, logging, paths, size_store, strip_order_store,
};
use airpane_strip::window::{Strip, enable_per_monitor_v2};

/// 模式切换快捷键的注册 id。
const TOGGLE_MODE_HOTKEY_ID: i32 = 1;
/// 预览区窗口的类名，用于检测已有实例。
const STRIP_CLASS_NAME: &str = "AirPaneStrip";

fn main() -> ExitCode {
    // 单实例：两个 AirPane 会互相抢窗口与快捷键。
    if find_window_by_class(STRIP_CLASS_NAME).is_some() {
        dialog::info(
            "AirPane",
            "AirPane 已经在运行了。\n\n如果它是以另一种权限启动的（比如普通权限与管理员权限各一个），请先在任务管理器里结束它。",
        );
        return ExitCode::SUCCESS;
    }

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

    let (action_tx, actions) = channel();
    let mut strip = match Strip::create(config.strip.width_dip, action_tx) {
        Ok(strip) => strip,
        Err(err) => {
            log_warn!("创建预览区失败: {err}");
            eprintln!("创建预览区失败: {err}");
            return ExitCode::FAILURE;
        }
    };

    let sizes = size_store::read(&paths::window_sizes_file()).unwrap_or_default();
    let order = strip_order_store::read(&paths::strip_order_file()).unwrap_or_default();
    let mut app = app::App::new(
        sizes,
        order,
        app::AppPaths {
            snapshot: paths::snapshot_file(),
            sizes: paths::window_sizes_file(),
            order: paths::strip_order_file(),
        },
    );

    let (monitor, events) = match EventMonitor::start() {
        Ok((monitor, events)) => (monitor, events),
        Err(err) => {
            log_warn!("事件监听启动失败: {err}");
            eprintln!("事件监听启动失败: {err}");
            return ExitCode::FAILURE;
        }
    };

    let hotkey = match Hotkey::register(&config.hotkey.toggle_mode, TOGGLE_MODE_HOTKEY_ID) {
        Ok(hotkey) => {
            log_info!("模式切换快捷键：{}", config.hotkey.toggle_mode);
            Some(hotkey)
        }
        Err(err) => {
            log_warn!("{err}");
            dialog::info(
                "AirPane",
                &format!(
                    "{err}\n\n可以改 %APPDATA%\\AirPane\\config.json 里的 hotkey.toggle_mode，或者点预览区底部的「桌面」按钮切换模式。"
                ),
            );
            None
        }
    };

    // D-028：没有上次记录时默认进入 Stage 模式。
    if config.startup.mode != StartupMode::Normal {
        app.enter_stage(&mut strip);
    }

    println!(
        "AirPane {} — {}",
        env!("CARGO_PKG_VERSION"),
        app.mode_name()
    );
    println!("配置：{}", config_path.display());
    println!("日志：{}", log_path.display());
    println!("模式切换：{}", config.hotkey.toggle_mode);

    message_loop::run(1000, |message| match message {
        LoopMessage::Wakeup => {
            while let Ok(event) = events.try_recv() {
                app.on_event(event, &mut strip);
            }
            while let Ok(action) = actions.try_recv() {
                app.on_action(action, &mut strip);
            }
        }
        LoopMessage::Timer => app.reconcile(&mut strip),
        LoopMessage::Hotkey(id) => {
            if id == TOGGLE_MODE_HOTKEY_ID {
                app.toggle_mode(&mut strip);
                println!("已切换到{}", app.mode_name());
            }
        }
    });

    drop(hotkey);
    drop(monitor);
    // `app` 在这里析构，Drop 会把移动过的窗口还原并清掉快照。
    drop(app);
    ExitCode::SUCCESS
}
