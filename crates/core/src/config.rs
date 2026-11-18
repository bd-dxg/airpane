//! 应用配置。
//!
//! 配置存于 `%APPDATA%\AirPane\config.json`（见 `docs/DECISIONS.md` D-029）。本模块
//! 只放数据结构、默认值与纯计算，读写由 `airpane-platform-win::config_io` 完成，
//! 因此可以在任意平台上单元测试。

use serde::{Deserialize, Serialize};

/// 配置文件格式版本。结构发生不兼容变化时递增。
pub const CONFIG_VERSION: u32 = 1;

/// 预览区宽度的下限、上限与默认值，单位为逻辑像素（DIP），来自 PRD 4.5。
pub const MIN_STRIP_WIDTH_DIP: u32 = 120;
pub const MAX_STRIP_WIDTH_DIP: u32 = 360;
pub const DEFAULT_STRIP_WIDTH_DIP: u32 = 200;

/// 预览区宽度最多占可用宽度的百分比。
///
/// D-012 裁定：PRD 4.5 的 6% 下限不生效，只用 25% 收紧上限。
pub const MAX_STRIP_WIDTH_PERCENT: u32 = 25;

/// 启动时进入的模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupMode {
    /// 沿用上次退出时的模式；没有记录时进入 Stage 模式（D-028）。
    #[default]
    Last,
    Stage,
    Normal,
}

/// 日志级别，声明顺序即严重程度递减（`Error` 最低，`Debug` 最高）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
}

impl LogLevel {
    /// 日志行里显示的级别名。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
        }
    }
}

/// 预览区配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StripConfig {
    /// 设定的宽度，单位为 DIP。实际生效值还要经过 [`effective_strip_width`]。
    pub width_dip: u32,
}

impl Default for StripConfig {
    fn default() -> Self {
        Self {
            width_dip: DEFAULT_STRIP_WIDTH_DIP,
        }
    }
}

/// 快捷键配置。字符串形如 `Ctrl+Alt+S`，解析在 `airpane-platform-win::hotkey`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// 模式切换快捷键，默认 Ctrl+Alt+S（PRD 4.2）。
    pub toggle_mode: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle_mode: "Ctrl+Alt+S".to_string(),
        }
    }
}

/// 启动配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupConfig {
    pub mode: StartupMode,
}

/// 日志配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: LogLevel,
}

/// 完整的应用配置。
///
/// 字段级 `serde(default)` 保证旧版本或手写的配置文件缺字段时仍能加载：
/// 缺失字段取默认值，未知字段被忽略，解析失败由调用方回退到默认配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 文件格式版本，写入时原样保留。缺字段的旧文件由 `Default` 补成当前版本。
    pub version: u32,
    pub strip: StripConfig,
    pub hotkey: HotkeyConfig,
    pub startup: StartupConfig,
    pub log: LogConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            strip: StripConfig::default(),
            hotkey: HotkeyConfig::default(),
            startup: StartupConfig::default(),
            log: LogConfig::default(),
        }
    }
}

/// 计算预览区实际生效的宽度，单位 DIP。
///
/// 设定值先钳制到 [`MIN_STRIP_WIDTH_DIP`] 与 [`MAX_STRIP_WIDTH_DIP`] 之间，再按可用
/// 宽度的 [`MAX_STRIP_WIDTH_PERCENT`] 收紧上限（D-012）。结果至少为 1 DIP，避免可用
/// 宽度极小时得到 0 导致预览区不可见。
pub fn effective_strip_width(width_dip: u32, available_width_dip: i32) -> u32 {
    let clamped = width_dip.clamp(MIN_STRIP_WIDTH_DIP, MAX_STRIP_WIDTH_DIP);
    let available = available_width_dip.max(0) as u32;
    let cap = available * MAX_STRIP_WIDTH_PERCENT / 100;
    clamped.min(cap).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_prd_defaults() {
        let config = AppConfig::default();
        assert_eq!(config.version, CONFIG_VERSION);
        assert_eq!(config.strip.width_dip, DEFAULT_STRIP_WIDTH_DIP);
        assert_eq!(config.hotkey.toggle_mode, "Ctrl+Alt+S");
        assert_eq!(config.startup.mode, StartupMode::Last);
        assert_eq!(config.log.level, LogLevel::Info);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let config: AppConfig = serde_json::from_str("{}").expect("空对象应当解析成功");
        assert_eq!(config, AppConfig::default());
        assert_eq!(
            config.version, CONFIG_VERSION,
            "缺 version 字段时应补成当前版本，而不是 0"
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let config: AppConfig =
            serde_json::from_str(r#"{"future_field": 42}"#).expect("多余字段不应导致解析失败");
        assert_eq!(config, AppConfig::default());
    }

    #[test]
    fn config_round_trips_through_json() {
        let original = AppConfig {
            version: CONFIG_VERSION,
            strip: StripConfig { width_dip: 264 },
            hotkey: HotkeyConfig {
                toggle_mode: "Ctrl+Shift+Space".to_string(),
            },
            startup: StartupConfig {
                mode: StartupMode::Stage,
            },
            log: LogConfig {
                level: LogLevel::Debug,
            },
        };
        let json = serde_json::to_string(&original).expect("序列化应当成功");
        let restored: AppConfig = serde_json::from_str(&json).expect("反序列化应当成功");
        assert_eq!(restored, original);
    }

    #[test]
    fn strip_width_is_clamped_to_prd_range() {
        assert_eq!(effective_strip_width(100, 2560), MIN_STRIP_WIDTH_DIP);
        assert_eq!(effective_strip_width(999, 2560), MAX_STRIP_WIDTH_DIP);
        assert_eq!(effective_strip_width(200, 2560), 200);
    }

    #[test]
    fn strip_width_is_capped_at_a_quarter_of_available_width() {
        // 480 的 25% 正好是 120，与下限重合。
        assert_eq!(effective_strip_width(200, 480), 120);
        // 320 的 25% 是 80，低于 PRD 的 120 下限；D-012 裁定上限优先。
        assert_eq!(effective_strip_width(200, 320), 80);
        // 1366 屏幕的 25% 是 341，不触发收紧。
        assert_eq!(effective_strip_width(200, 1366), 200);
    }

    #[test]
    fn extreme_available_width_never_returns_zero() {
        assert_eq!(effective_strip_width(200, 0), 1);
        assert_eq!(effective_strip_width(200, -100), 1);
    }
}
