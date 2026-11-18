//! 配置文件（JSON）的读写。
//!
//! 配置文件缺失或损坏时回退到默认值并写回磁盘，保证下一次启动有一份可编辑的样本。
//! 写入用「临时文件 + 改名」，避免中途退出留下半截 JSON。

use std::fs;
use std::io;
use std::path::Path;

use airpane_core::config::AppConfig;

/// 读取配置；文件缺失或损坏时写回默认配置。
///
/// 损坏的文件先重命名为 `config.json.broken`，避免把用户手写的内容直接覆盖掉。
/// 第二个返回值是需要写进日志的提示，正常加载时为 `None`。
pub fn load_or_create(path: &Path) -> (AppConfig, Option<String>) {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<AppConfig>(&text) {
            Ok(config) => (config, None),
            Err(err) => {
                let config = AppConfig::default();
                let backup = path.with_extension("json.broken");
                let _ = fs::rename(path, &backup);
                let notice = match save(path, &config) {
                    Ok(()) => format!(
                        "配置文件解析失败，原文件已备份到 {} 并重置为默认值: {err}",
                        backup.display()
                    ),
                    Err(save_err) => {
                        format!("配置文件解析失败（{err}），且写入默认值失败: {save_err}")
                    }
                };
                (config, Some(notice))
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let config = AppConfig::default();
            let notice = save(path, &config).err().map(|err| {
                format!(
                    "配置文件不存在，写入默认值失败（{}）: {err}",
                    path.display()
                )
            });
            (config, notice)
        }
        Err(err) => {
            let notice = format!("读取配置失败，使用默认值（{}）: {err}", path.display());
            (AppConfig::default(), Some(notice))
        }
    }
}

/// 原子写入配置：先写 `config.json.tmp`，再改名覆盖正式文件。
pub fn save(path: &Path, config: &AppConfig) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text)?;
    // Windows 上 `fs::rename` 使用 MoveFileEx(MOVEFILE_REPLACE_EXISTING)，可直接覆盖。
    fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use airpane_core::config::{LogLevel, StartupMode};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("airpane-config-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_creates_default_config() {
        let dir = temp_dir("missing");
        let path = dir.join("config.json");

        let (config, notice) = load_or_create(&path);
        assert_eq!(config, AppConfig::default());
        assert!(notice.is_none(), "首次创建不应产生警告: {notice:?}");
        assert!(path.exists(), "默认配置应当写回磁盘");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_file_is_backed_up_and_replaced_with_defaults() {
        let dir = temp_dir("broken");
        let path = dir.join("config.json");
        fs::create_dir_all(&dir).expect("创建测试目录");
        fs::write(&path, "{ 不是合法 JSON").expect("写入损坏文件");

        let (config, notice) = load_or_create(&path);
        assert_eq!(config, AppConfig::default());
        assert!(notice.is_some(), "损坏文件应当产生提示");

        let backup = path.with_extension("json.broken");
        assert_eq!(
            fs::read_to_string(&backup).expect("应当保留原文件"),
            "{ 不是合法 JSON"
        );

        let restored: AppConfig =
            serde_json::from_str(&fs::read_to_string(&path).expect("读取配置"))
                .expect("应当可解析");
        assert_eq!(restored, AppConfig::default());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_config_is_loaded_back() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("config.json");
        let original = AppConfig {
            strip: airpane_core::config::StripConfig { width_dip: 264 },
            startup: airpane_core::config::StartupConfig {
                mode: StartupMode::Stage,
            },
            log: airpane_core::config::LogConfig {
                level: LogLevel::Debug,
            },
            ..AppConfig::default()
        };

        save(&path, &original).expect("保存配置");
        let (loaded, notice) = load_or_create(&path);

        assert_eq!(loaded, original);
        assert!(notice.is_none());
        assert!(
            !path.with_extension("json.tmp").exists(),
            "临时文件应当被改名"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
