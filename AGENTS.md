# AirPane（轻窗）开发指南

## 项目概览

Windows 上的「台前调度」式窗口管理工具：当前窗口留在中间，其余窗口以 DWM 实时缩略图常驻屏幕左侧，点击即切换。当前处于 **M0 技术原型阶段**，没有可用界面，接口与行为都可能大幅变化。

## 技术栈

- 语言：Rust stable（edition 2024，MSRV 1.85），目标 `x86_64-pc-windows-msvc`
- 系统 API：`windows` crate 0.62.2（Win32 / DWM / GDI / HiDpi）
- 序列化：`serde` + `serde_json`
- CI：GitHub Actions（`windows-latest`）

## 常用命令

```powershell
cargo build --workspace                                   # 构建
cargo test --workspace                                    # 单元测试
cargo clippy --workspace --all-targets -- -D warnings     # 静态检查（CI 门禁）
cargo fmt --all --check                                   # 格式检查（CI 门禁）
cargo run --bin airpane                                   # 主程序（当前只打印版本）
```

诊断与验收用的示例程序（都需要在真实桌面下跑，CI 不覆盖）：

```powershell
cargo run -p airpane-platform-win --example list_windows -- --all        # 窗口列表 + 每条被排除的原因
cargo run -p airpane-platform-win --example snapshot_check               # 写快照并挪开窗口，验证启动恢复
cargo run -p airpane-platform-win --example watch_events -- --seconds 30 # 事件同步一致性，0 不一致才退出码 0
cargo run -p airpane-platform-win --example switch_check -- --rounds 200 # 前台切换成功率，低于 99% 退出码 1
cargo run -p airpane-platform-win --example idle_cpu -- --seconds 20        # 运行中 AirPane 的空闲 CPU 与窗口数，20 窗口 ≥ 1% 非零退出
cargo run -p airpane-strip --example preview_strip -- --seconds 30       # 预览区 + 缩略图 + 点击切换
cargo run -p airpane-anim-prototype -- 300 [--no-flush]                  # 动画帧耗时与 CPU
```

## 目录结构

| 路径 | 职责 |
| --- | --- |
| `crates/core` | 纯逻辑：窗口过滤规则、窗口注册表、快照结构、布局计算。**不依赖 Win32**，必须可单元测试 |
| `crates/platform-win` | Win32 封装：枚举、WinEvent、前台切换、DPI、快照接口。`unsafe` 集中在这里 |
| `crates/strip` | 预览区窗口：窄栏、DWM 缩略图、点击切换。`unsafe` 集中在这里 |
| `crates/app` | 主程序，M0 阶段暂为占位 |
| `prototypes/anim` | M0 的一次性动画原型，验证完废弃，M1 不继承 |
| `docs/` | `PRD.md`（冻结的草案基线）、`DECISIONS.md`（冲突时以此为准）、`DEPENDENCIES.md`、`CHANGELOG.md`、M0 的两份报告 |

## 代码约定

- `unsafe` 只允许出现在 `platform-win` 与 `strip`，每个 `unsafe` 块前写明安全前提。
- `core` 里的东西必须能在任意平台单测；需要 Win32 的地方用 `WindowFacts` 这类纯数据入参。
- 新增依赖先在 `docs/DEPENDENCIES.md` 登记版本、许可证，并确认与 `GPL-3.0-only` 兼容。
- 与 PRD 不一致的取舍写进 `docs/DECISIONS.md`（追加编号，不改历史条目）。
- 提交必须带 `Signed-off-by`（`git commit -s`）；一个 PRD 任务对应一个分支与一个 PR，明显过大的任务拆子 PR。
- 合并前 `cargo fmt --all --check`、`clippy -- -D warnings`、`test`、`build` 必须全绿（CI 会自动跑）。
- 文档与注释用中文，标识符用英文。

## 注意事项

### 硬约束（来自 PRD 11.3，不能违反）

1. 不向其他进程注入代码、不 hook 其内部；事件监听只用 `WINEVENT_OUTOFCONTEXT`。
2. 快捷键只用 `RegisterHotKey`，不用低级键盘钩子，不记录按键。
3. 移动或缩放他人窗口前，必须先把原状态落盘（快照层）。
4. 全屏/游戏规则命中时，不得对触发窗口做任何移动、缩放、置顶、抢焦点操作。
5. 窗口画面只用于 DWM 缩略图实时显示，不截图落盘、不上传。
6. 不联网，唯一例外是检查并下载更新。

### 已经踩过的坑（不要重复）

- **`EVENT_OBJECT_DESTROY` 不会投递**（进程退出时窗口消失没有事件）。事件必须配 1Hz 的 `reconcile_registry` 校准，见 D-020。
- **记事本等应用会创建临时可见的辅助窗口**（`Non Client Input Sink Window`、空标题）。过滤必须包含「顶层窗口」与「owner 窗口除 `WS_EX_APPWINDOW` 外排除」，见 D-021。
- **`AttachThreadInput` 要挂到当前前台窗口的线程**，挂目标窗口的线程无法绕过前台锁（实测 36.67% vs 100%）。
- **DWM 缩略图不参与目标窗口内部的视觉树**，任何 D2D/DComp 变换都不会影响它；动画只能改覆盖层窗口的矩形。
- `windows` 0.62 里 `BOOL` 在 `windows::core::BOOL`；`HTHUMBNAIL` 在绑定中是 `isize`；`EVENT_*` 常量在 `WindowsAndMessaging`，而 `SetWinEventHook` 在 `UI::Accessibility`。

### 环境限制

开发机是单屏 2560×1440 @100%、Intel Iris Xe、Windows 11 build 26200。**没有 Win10、没有第二块屏、没有 150% 缩放环境**，相关验收一律标注为缺口，不要假装验证过。

## 当前状态

- M0（T0.1–T0.6）已全部完成并合并。
- M1（T1.1–T1.8）功能已全部合并；待办只剩验收：3 天日常使用（`docs/test-report-M1.md` 第 2.7 节的结论待补）。
- 主程序默认进入 Stage 模式，`Ctrl+Alt+S` 或预览区底部的「桌面」按钮切换模式；切回普通桌面时全部窗口还原。
- M1 范围与验收结果见 `docs/test-report-M1.md`；已知缺口见其第 4 节。
- 开工前建议先读 `docs/DECISIONS.md`，它是与 PRD 冲突时的唯一真相源；写窗口相关代码前必读 D-030（位置尺寸统一用 DWM 可见边界）、D-022（只移动活跃窗口）与 D-034（最小化窗口先同步还原）。
