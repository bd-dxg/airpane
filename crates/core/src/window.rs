//! 窗口过滤规则。
//!
//! 规则来自 PRD 4.1 与 5.3：只管理当前虚拟桌面上、属于普通应用的可见顶层窗口。
//! 本模块只做纯逻辑判定，Win32 查询结果由 `airpane-platform-win` 填进 [`WindowFacts`]，
//! 因此可以在任意平台上单元测试。

use core::fmt;

/// 系统壳窗口的类名：任务栏、桌面、开始菜单宿主与任务视图等。
///
/// 注意：`Windows.UI.Core.CoreWindow` 不在此列。该类型的实际使用上下文（XAML 岛、
/// 输入宿主）已被不可见与 cloak 规则滤掉，而保留它可能误杀采用该类的 UWP 应用窗口。
const SHELL_CLASS_NAMES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "ForegroundStaging",
    "MultitaskingViewFrame",
    "XamlExplorerHostIslandWindow",
];

/// 系统 UI 宿主的进程名。这些窗口看起来像普通顶层窗口，但它们是开始菜单、搜索、
/// 音量/网络面板、输入法候选框等，搬走会让系统 UI 坏掉。
const SHELL_PROCESS_NAMES: &[&str] = &[
    "StartMenuExperienceHost.exe",
    "SearchHost.exe",
    "ShellExperienceHost.exe",
    "TextInputHost.exe",
    "LockApp.exe",
];

/// 判定一个窗口是否可被管理所需的事实。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowFacts {
    /// `IsWindowVisible` 的结果。最小化的窗口仍然可见，应当保留。
    pub visible: bool,
    /// 带 `WS_EX_TOOLWINDOW` 扩展样式。
    pub tool_window: bool,
    /// 带 `WS_EX_APPWINDOW` 扩展样式：即使有 owner 也应视为独立应用窗口。
    pub app_window: bool,
    /// 有 owner 窗口：通常是对话框、弹出窗或控件辅助窗口。
    pub owned: bool,
    /// `DWMWA_CLOAKED` 非零：属于其他虚拟桌面，或被挂起的 UWP 窗口。
    pub cloaked: bool,
    /// 窗口类名。
    pub class_name: String,
    /// 进程可执行文件名。UWP 窗口填的是解析后的真实应用进程。
    pub process_name: String,
    /// 是否是顶层窗口（`GetAncestor(GA_ROOT)` 就是它自己）。
    pub top_level: bool,
    /// 属于 AirPane 自己的进程。
    pub own_process: bool,
}

/// 窗口被排除的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// 拿不到类名，通常意味着句柄已经失效。
    EmptyClassName,
    /// 子窗口或属于其他窗口层级，不属于预览区的管理范围。
    NotTopLevel,
    Invisible,
    Cloaked,
    ToolWindow,
    /// 有 owner 且未标 `WS_EX_APPWINDOW`：对话框、弹出窗或应用自建的辅助窗口。
    OwnedPopup,
    OwnProcess,
    ShellWindow,
    /// 系统 UI 宿主进程：开始菜单、搜索、输入法候选框等。
    ShellProcess,
}

impl fmt::Display for RejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::EmptyClassName => "没有类名",
            Self::NotTopLevel => "不是顶层窗口",
            Self::Invisible => "不可见",
            Self::Cloaked => "已被 DWM cloak",
            Self::ToolWindow => "工具窗口",
            Self::OwnedPopup => "附属弹窗",
            Self::OwnProcess => "属于 AirPane 自己",
            Self::ShellWindow => "系统壳窗口",
            Self::ShellProcess => "系统 UI 宿主窗口",
        };
        f.write_str(text)
    }
}

/// 判断窗口是否可被管理。`Ok(())` 表示进入预览区。
pub fn classify(facts: &WindowFacts) -> Result<(), RejectReason> {
    if facts.class_name.is_empty() {
        return Err(RejectReason::EmptyClassName);
    }
    if facts.own_process {
        return Err(RejectReason::OwnProcess);
    }
    if !facts.top_level {
        return Err(RejectReason::NotTopLevel);
    }
    if !facts.visible {
        return Err(RejectReason::Invisible);
    }
    if facts.cloaked {
        return Err(RejectReason::Cloaked);
    }
    if facts.tool_window {
        return Err(RejectReason::ToolWindow);
    }
    if facts.owned && !facts.app_window {
        return Err(RejectReason::OwnedPopup);
    }
    if is_shell_window(&facts.class_name) {
        return Err(RejectReason::ShellWindow);
    }
    if is_shell_process(&facts.process_name) {
        return Err(RejectReason::ShellProcess);
    }
    Ok(())
}

fn is_shell_window(class_name: &str) -> bool {
    SHELL_CLASS_NAMES
        .iter()
        .any(|shell| shell.eq_ignore_ascii_case(class_name))
}

fn is_shell_process(process_name: &str) -> bool {
    SHELL_PROCESS_NAMES
        .iter()
        .any(|shell| shell.eq_ignore_ascii_case(process_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal() -> WindowFacts {
        WindowFacts {
            visible: true,
            tool_window: false,
            app_window: false,
            owned: false,
            cloaked: false,
            class_name: "Notepad".to_string(),
            process_name: "notepad.exe".to_string(),
            top_level: true,
            own_process: false,
        }
    }

    #[test]
    fn normal_window_is_managed() {
        assert_eq!(classify(&normal()), Ok(()));
    }

    #[test]
    fn minimized_window_stays_visible() {
        // 最小化不会让 IsWindowVisible 变假，因此仍然被管理。
        let facts = WindowFacts {
            visible: true,
            ..normal()
        };
        assert_eq!(classify(&facts), Ok(()));
    }

    #[test]
    fn rejects_invisible_window() {
        let facts = WindowFacts {
            visible: false,
            ..normal()
        };
        assert_eq!(classify(&facts), Err(RejectReason::Invisible));
    }

    #[test]
    fn rejects_cloaked_window() {
        let facts = WindowFacts {
            cloaked: true,
            ..normal()
        };
        assert_eq!(classify(&facts), Err(RejectReason::Cloaked));
    }

    #[test]
    fn rejects_tool_window() {
        let facts = WindowFacts {
            tool_window: true,
            ..normal()
        };
        assert_eq!(classify(&facts), Err(RejectReason::ToolWindow));
    }

    #[test]
    fn rejects_shell_windows_case_insensitively() {
        for class_name in ["Shell_TrayWnd", "progman", "WorkerW"] {
            let facts = WindowFacts {
                class_name: class_name.to_string(),
                ..normal()
            };
            assert_eq!(
                classify(&facts),
                Err(RejectReason::ShellWindow),
                "{class_name}"
            );
        }
    }

    #[test]
    fn rejects_shell_processes_case_insensitively() {
        for process_name in [
            "StartMenuExperienceHost.exe",
            "searchhost.exe",
            "ShellExperienceHost.exe",
            "TextInputHost.exe",
        ] {
            let facts = WindowFacts {
                process_name: process_name.to_string(),
                ..normal()
            };
            assert_eq!(
                classify(&facts),
                Err(RejectReason::ShellProcess),
                "{process_name}"
            );
        }
    }

    #[test]
    fn rejects_child_window() {
        let facts = WindowFacts {
            top_level: false,
            ..normal()
        };
        assert_eq!(classify(&facts), Err(RejectReason::NotTopLevel));
    }

    #[test]
    fn rejects_owned_popup_unless_app_window() {
        let popup = WindowFacts {
            owned: true,
            ..normal()
        };
        assert_eq!(classify(&popup), Err(RejectReason::OwnedPopup));

        let app = WindowFacts {
            owned: true,
            app_window: true,
            ..normal()
        };
        assert_eq!(
            classify(&app),
            Ok(()),
            "带 WS_EX_APPWINDOW 的 owner 窗口仍应管理"
        );
    }

    #[test]
    fn rejects_own_process_and_empty_class() {
        let own = WindowFacts {
            own_process: true,
            ..normal()
        };
        assert_eq!(classify(&own), Err(RejectReason::OwnProcess));

        let empty = WindowFacts {
            class_name: String::new(),
            ..normal()
        };
        assert_eq!(classify(&empty), Err(RejectReason::EmptyClassName));
    }

    #[test]
    fn application_frame_window_is_managed() {
        let facts = WindowFacts {
            class_name: "ApplicationFrameWindow".to_string(),
            ..normal()
        };
        assert_eq!(classify(&facts), Ok(()));
    }
}
