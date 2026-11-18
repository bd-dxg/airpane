# 变更日志

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循语义化版本。

## [Unreleased]

### 新增（M1 自用版）

- 配置文件 `%APPDATA%\AirPane\config.json`：预览区宽度、模式切换快捷键、启动模式、日志级别；缺失时自动重建，损坏时保留为 `config.json.broken`
- 预览区宽度生效值计算：120–360 DIP 钳制，再按可用宽度 25% 收紧上限（D-012）
- 自写文件日志：级别过滤、1MB × 3 份滚动、本地时间戳，写入 `%APPDATA%\AirPane\logs\airpane.log`（D-027）
- 主程序启动接入配置与日志，打印生效的路径与配置项
- 窗口快照格式 v2：增加 `clean_exit` 标记与窗口类名，带跨重启配对算法（hwnd 优先，回退进程名 + 类名 + 矩形就近）
- 快照原子读写：`%APPDATA%\AirPane\snapshot.json`，版本不符或损坏时忽略
- 移动窗口的唯一入口 `platform-win::placement`：按 DWM 可见边界摆位，带阴影补偿，不抢焦点、不改 Z 序
- 崩溃恢复：启动时读到未清理的快照就把窗口摆回原位，处理完清除快照（D-024）
- 窗口位置与尺寸计算统一使用 `DWMWA_EXTENDED_FRAME_BOUNDS`（D-030）
- 新示例 `snapshot_check`：写快照并挪开窗口，用于验证启动恢复
- Stage 布局：舞台区 = 工作区减去预览区，活跃窗口移入舞台区，其余窗口原地不动（D-019、D-022）
- 活跃窗口跟随前台窗口（D-023）；最大化/全屏窗口不搬动（D-026）
- 尺寸记忆：`%APPDATA%\AirPane\window-sizes.json`，占满舞台区的尺寸不记录（D-025）
- 预览区只列非活跃窗口，宽度改从配置读取并应用 D-012 钳制（D-032）
- 系统 UI 宿主进程（开始菜单、搜索、输入法候选框、搜索）不进入预览区（D-031）
- 退出（含 panic unwind）时还原所有被移动过的窗口并清除快照（D-024）
- 新增主线程消息循环：事件线程用 `PostThreadMessageW` 唤醒，另带 1Hz 校准定时器
- 模式切换快捷键：默认 `Ctrl+Alt+S`，只走 `RegisterHotKey`（PRD 11.3 第 2 条）
- 预览区底部 40 DIP 工具区：「设置」（M1 弹提示）与「桌面」两个按钮
- 缩略图列表支持鼠标滚轮滚动，工具区固定在最下方
- 切换模式时：普通桌面模式隐藏预览区并还原全部窗口，再切回 Stage 时重新搬入
- 最小化窗口：先同步还原再搬入，尺寸记忆只记普通状态的窗口（D-034）
- 空闲 CPU 优化：1Hz 校准只校已跟踪窗口，每 10 秒全量枚举兜底，预览区列表无变化时不碰 DWM（D-035）
- 新示例 `idle_cpu`：采样运行中 AirPane 的空闲 CPU 占用与可管理窗口数
- 新增 `docs/test-report-M1.md`：M1 的用例、结果、与 M0 的差异、已知缺口
- 桌面成为前台（Win+D、显示桌面）时自动隐藏预览区，切回窗口后恢复（D-036）
- 最大化窗口在 Stage 模式下摆进舞台区，切回普通桌面时恢复最大化状态；非最大化的真全屏仍不搬（D-037）
- 预览区光标改为标准的箭头，不再继承其他窗口的等待光标（D-038）

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
