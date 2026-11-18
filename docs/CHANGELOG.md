# 变更日志

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循语义化版本。

## [Unreleased]

### 新增（M1 自用版）

- 配置文件 `%APPDATA%\AirPane\config.json`：预览区宽度、模式切换快捷键、启动模式、日志级别；缺失时自动重建，损坏时保留为 `config.json.broken`
- 预览区宽度生效值计算：120–360 DIP 钳制，再按可用宽度 25% 收紧上限（D-012）
- 自写文件日志：级别过滤、1MB × 3 份滚动、本地时间戳，写入 `%APPDATA%\AirPane\logs\airpane.log`（D-027）
- 主程序启动接入配置与日志，打印生效的路径与配置项

### 新增（M0 技术原型）

- Cargo workspace 骨架：`airpane-core`、`airpane-platform-win`、`airpane-strip`、`airpane`
- 窗口快照数据结构（`Rect`、`ShowState`、`WindowSnapshot`、`SnapshotFile`）与 JSON 往返测试
- 窗口枚举与过滤：顶层窗口、不可见、DWM cloak、工具窗口、owner 附属窗口、系统壳窗口、UWP 真实进程解析
- 窗口模型 `WindowRegistry`：按最近使用顺序（MRU）维护被管理窗口
- WinEvent 监听：前台切换、创建/销毁/显示/隐藏、标题变化、最小化/还原，事件经 channel 交给调用方
- 1Hz 校准：`reconcile_registry` 修复事件丢失导致的漂移（D-020）
- 诊断示例：`list_windows`（窗口列表与排除原因）、`watch_events`（事件同步一致性验收）
- 预览区：贴左侧的置顶窄栏、DWM 实时缩略图、点击缩略图切换窗口
- 动画路线原型（`prototypes/anim`）与实测报告：`docs/m0-animation-report.md`、`docs/test-report-M0.md`
- Win32 最小封装：`is_window` 句柄有效性查询
- GitHub Actions：格式检查、Clippy、测试、构建；PR 的 DCO 校验
- 中文 README、依赖清单（`docs/DEPENDENCIES.md`）、决策记录（`docs/DECISIONS.md`）
