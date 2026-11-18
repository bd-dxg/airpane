//! JSON 状态文件的原子写入。
//!
//! 先写临时文件再改名，保证进程中途退出时读到的要么是完整文件、要么是旧版本。

use std::fs;
use std::io;
use std::path::Path;

use serde::Serialize;

/// 原子写入一个 JSON 文件。
pub fn write_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text)?;
    // Windows 上 `fs::rename` 使用 MoveFileEx(MOVEFILE_REPLACE_EXISTING)，可直接覆盖。
    fs::rename(&temp, path)
}
