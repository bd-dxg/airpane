//! 窗口标题的展示规则（纯逻辑，可单测）。
//!
//! 预览区只有 200 DIP 宽，塞不下完整标题，只取最有辨识度的一段（D-044）。

/// 标题分段的分隔符。中文 Windows 上资源管理器等应用同样用它。
const SEPARATOR: &str = " - ";

/// 把窗口标题压成一行短标题。
///
/// 规则：按 `" - "` 分段（去掉空段）后，≥ 3 段取倒数第二段，否则取第一段。
/// `"main.rs - airpane - Visual Studio Code"` → `"airpane"`，
/// `"下载 - 文件资源管理器"` → `"下载"`。结果为空（标题为空或只有分隔符）时
/// 退回进程名。
pub fn short_title(title: &str, process_name: &str) -> String {
    let parts: Vec<&str> = title
        .split(SEPARATOR)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();

    let picked = if parts.len() >= 3 {
        parts[parts.len() - 2]
    } else {
        parts.first().copied().unwrap_or("")
    };

    if picked.is_empty() {
        process_name.to_string()
    } else {
        picked.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_single_segment_title() {
        assert_eq!(short_title("微信", "WeChat.exe"), "微信");
    }

    #[test]
    fn takes_first_segment_when_only_two() {
        assert_eq!(short_title("下载 - 文件资源管理器", "explorer.exe"), "下载");
    }

    #[test]
    fn takes_second_to_last_when_three_or_more() {
        assert_eq!(
            short_title("main.rs - airpane - Visual Studio Code", "Code.exe"),
            "airpane"
        );
        assert_eq!(
            short_title("a.rs - src - airpane - Visual Studio Code", "Code.exe"),
            "airpane"
        );
    }

    #[test]
    fn falls_back_to_process_name_when_title_is_empty() {
        assert_eq!(short_title("", "Code.exe"), "Code.exe");
        assert_eq!(short_title(" - ", "Code.exe"), "Code.exe");
        assert_eq!(short_title("   ", "Code.exe"), "Code.exe");
    }

    #[test]
    fn ignores_empty_segments() {
        assert_eq!(short_title("a -  - b - c", "app.exe"), "b");
    }

    #[test]
    fn trims_segments() {
        assert_eq!(short_title("  a - b  ", "app.exe"), "a");
    }
}
