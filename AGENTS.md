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

## 提交前必须走完的流程

1. `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全绿；顺手确认没有新增超过 300 行的源码文件。
2. 执行 `code-review-expert` skill 审查本次 diff。**按技能本身的流程走，不是写一句「自查通过」**：
   - 先读 `SKILL.md` 与它的三份 checklist（`solid-checklist.md`、`code-quality-checklist.md`、`security-checklist.md`），按它们逐项比。
   - 用 `git diff main...HEAD` 逐文件过，按 P0 / P1 / P2 / P3 分级；涉及窗口操作的改动要用真实窗口验，不能只靠编译通过。
   - **P0/P1/P2 必须修完并重跑第 1 步**；P3 要么修，要么在 PR 里写清不修的理由。
   - 审查结论原文（问题位置、现象、修法）与「未覆盖项」（本机没有的环境、留到后续的验证）都要写进 PR 描述。审查发现的问题单独提交时要能对应上。
   - 确认不修的问题必须写明原因，不允许闷着不提。
3. 执行 `gencom` skill 生成提交信息，提交带 `Signed-off-by`（`git commit -s`）。

## 代码文件大小

- 单个源码文件不超过 **300 行**（含注释与测试）；超出就按职责拆成子模块，用目录形式与父模块同名，例如 `crates/strip/src/window/`。
- 拆分只移动代码、不改行为；拆完必须重跑测试，纯逻辑要保证单元测试仍覆盖原函数。
- `prototypes/` 下的一次性原型不受此限制（M1 弃用，不再维护）。

## 目录结构

| 路径 | 职责 |
| --- | --- |
| `crates/core` | 纯逻辑：窗口过滤规则、窗口注册表、快照结构、布局计算。**不依赖 Win32**，必须可单元测试 |
| `crates/platform-win` | Win32 封装：枚举、WinEvent、前台切换、DPI、快照接口。`unsafe` 集中在这里 |
| `crates/strip` | 预览区窗口：窄栏、DWM 缩略图、点击切换。`unsafe` 集中在这里；`window/` 下拆出布局、绘制、消息处理与状态 |
| `crates/app` | 主程序：配置与日志、消息循环、Stage 控制器；`app/` 下按搬窗口、快照与尺寸、观察期、注册表拆分 |
| `assets/` | 应用图标：`airpane.ico` 与 `airpane.rc`，由 `crates/app/build.rs` 用 `rc.exe` 编进 exe |
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
- **分层窗口与 Acrylic 都做不出「缩略图清晰 + 背景通透」**：`WS_EX_LAYERED` 的 alpha 整窗生效，缩略图 `opacity` 上限 255 无法补偿；`WS_THICKFRAME` + `DwmExtendFrameIntoClientArea` + `DWMSBT_TRANSIENTWINDOW` 全部返回 Ok 但画面不变。macOS 台前调度用的是自绘合成，等价实现需要 DirectComposition（见 D-043）。
- **`mmc.exe`（任务计划程序）用 `WS_POPUP` 自定义最大化**：`IsZoomed` 返回 false 但铺满整屏，不能只看「覆盖整屏」就当成全屏游戏（见 D-041）。
- **提权窗口在普通权限下改不动**：`SetWindowPos` 返回「拒绝访问」（UIPI）。日志表现为 `移动 mmc.exe 失败：拒绝访问`，这不是 bug；验证提权路径要 `Start-Process .\target\release\airpane.exe -Verb RunAs`。
- **验收前必须重新构建可执行文件**：`cargo clippy` 与 `cargo test` 不会更新 `target\debug\airpane.exe`，直接 `Start-Process` 跑旧二进制会得出错误结论。改完先 `cargo build --bin airpane`（或 `--release`）。
- **测之前清干净旧实例**：两个 AirPane 会互抢窗口与 `Ctrl+Alt+S`。已有单实例检测（D-042）会弹提示并退出，但仍建议先 `Get-Process airpane | Stop-Process`。
- `windows` 0.62 里 `BOOL` 在 `windows::core::BOOL`；`HTHUMBNAIL` 在绑定中是 `isize`；`EVENT_*` 常量在 `WindowsAndMessaging`，而 `SetWinEventHook` 在 `UI::Accessibility`；`LoadCursorW` 的 `IDC_ARROW` 也在 `WindowsAndMessaging`；`GET_WHEEL_DELTA_WPARAM` 宏没有绑定，需手动从 `wparam` 高 16 位取。
- `SHGetFileInfoW` 同时被 `Win32_UI_Shell` 与 `Win32_Storage_FileSystem` 两个 feature 门控，只开前者编不过；`TrackMouseEvent` 与 `TME_LEAVE` 在 `UI::Input::KeyboardAndMouse`，而 `WM_MOUSELEAVE` 只在 `UI::Controls`（为一个常量不值得加 feature，手动定义 0x02A3）。

### 环境限制

开发机是单屏 2560×1440 @100%、Intel Iris Xe、Windows 11 build 26200。**没有 Win10、没有第二块屏、没有 150% 缩放环境**，相关验收一律标注为缺口，不要假装验证过。

## 当前状态

- M0（T0.1–T0.6）已全部完成并合并。
- M1（T1.1–T1.8）功能已全部合并，并完成六轮试用反馈修复（行为修复 D-036 ~ D-042、预览区 UI D-044 ~ D-046、固定排序与大图标条 D-047 ~ D-049、应用图标 D-050、当前窗口入列 D-051、全屏隐藏 D-052、拖动排序 D-053）；待办是 3 天日常使用验收。
- release 构建不申请控制台窗口，启动信息看 `%APPDATA%\AirPane\logs\airpane.log`；debug 构建保留控制台。
- 主程序默认进入 Stage 模式，`Ctrl+Alt+S` 或预览区底部的「桌面」按钮切换模式；切回普通桌面时全部窗口还原。
- M1 范围与验收结果见 `docs/test-report-M1.md`；已知缺口见其第 4 节。
- 开工前建议先读 `docs/DECISIONS.md`，它是与 PRD 冲突时的唯一真相源；写窗口相关代码前必读 D-030（位置尺寸统一用 DWM 可见边界）、D-022（只移动活跃窗口）、D-034（最小化窗口先同步还原）、D-039/D-040（尺寸记忆与最大化/启动的交互）。
