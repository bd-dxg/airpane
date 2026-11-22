# AirPane（轻窗）

Windows 上的「台前调度」式窗口管理：当前窗口待在舞台区，其余窗口以缩略图常驻一侧，点击即切换。窗口位置固定可预期，减少 Alt+Tab 里逐个找窗口的成本。

> **状态：v0.1.0（M1 自用版）已发布，项目停止开发。** 缘由与保留范围见 [`D-059`](docs/DECISIONS.md)。功能完整、可长期自用；没有设置界面（配置只能改 `config.json`），没有开机自启与托盘图标（原计划在 M3，不再实现）。欢迎 fork 接手。

## 现在能做什么

- **台前调度**：屏幕分成左侧预览区与右侧舞台区。前台窗口自动摆进舞台区，其余窗口原地不动。
- **预览区即当前所有窗口**：固定顺序（进程名 + 类名 + 句柄），**支持拖动排序**，顺序保存到磁盘、重启后保持；当前在舞台区的窗口也列出来并用浅蓝强调条标出，切换时列表内容与位置完全不变。
- **大图标条**：每个缩略图下方 40 DIP 的应用图标（三级兜底：窗口图标 → 类图标 → exe 文件图标）；配色跟随系统浅色/深色主题。
- **切回即还原**：`Ctrl+Alt+S` 或预览区底部的「桌面」按钮切回普通桌面时，所有被本程序移动过的窗口恢复原位置、原尺寸与最大化状态。
- **崩溃恢复**：移动任何窗口之前先落盘快照。强杀后重启会把窗口摆回原位。
- **全屏零打扰**：当前窗口盖住整屏（视频全屏、浏览器 F11、游戏）时预览区自动隐藏，退出后几十毫秒内恢复；桌面成为前台（Win+D）时同样隐藏。
- **尺寸记忆**：记住你给每个窗口调过的尺寸，下次进舞台区按记忆居中；最大化与满屏尺寸不记录。
- **任务栏任意边**：预览区贴工作区的边，任务栏在上下左右任意一边都不会被遮住。
- **不注入、不挂钩**：只读窗口信息，不向任何进程注入代码，不安装键盘钩子。

## 没做的（不再实现）

以下都是原定的 M2 及之后范围，随项目停止开发一并搁置，留作接手者的起点：

- 冲突规避的完整规则（进程名单、`SHQueryUserNotificationState`）、窗口分组、设置面板、登录自启、托盘图标、自动更新、多显示器。
- 预览区的 Mac 式「缩略图清晰 + 背景通透」需要重写渲染层（DirectComposition）。
- 40 DIP 图标条与系统原生侧边任务栏功能重叠，但因为没有后续改动，不做删减。

## 快速开始

需要 Windows 11（M1 只在 Win11 build 26200 上验过，Win10 未测）与 Rust stable（`x86_64-pc-windows-msvc`），另外需要 Windows SDK 的 `rc.exe`（MSVC 工具链默认自带，用来把应用图标编进 exe）。

```powershell
cargo build --release --bin airpane
.\target\release\airpane.exe
```

启动后直接进入 Stage 模式：

- `Ctrl+Alt+S` 在 Stage 与普通桌面之间切换；预览区底部也有「设置」与「桌面」两个按钮
- 预览区：滚轮滚动列表、点击切窗口、按住拖动改顺序
- 单实例：已经有 AirPane 在跑时，第二个实例会弹提示并退出

配置与状态文件都在 `%APPDATA%\AirPane\`：

| 文件 | 内容 |
| --- | --- |
| `config.json` | 预览区宽度、模式切换快捷键、启动模式、日志级别 |
| `snapshot.json` | 崩溃恢复用的窗口快照（正常退出会删除） |
| `window-sizes.json` | 每个窗口的尺寸记忆 |
| `strip-order.json` | 手动拖动出来的预览区顺序 |
| `logs\airpane.log` | 日志（release 版没有控制台，只能看这里） |

## 构建与诊断

```powershell
cargo build --workspace                                   # 构建
cargo test --workspace                                    # 单元测试（89 个）
cargo clippy --workspace --all-targets -- -D warnings     # 静态检查（CI 门禁）
cargo fmt --all --check                                   # 格式检查（CI 门禁）
```

诊断与验收示例（都需要在真实桌面下跑，CI 不覆盖）：

```powershell
cargo run -p airpane-platform-win --example list_windows -- --all        # 窗口列表与每条排除原因
cargo run -p airpane-platform-win --example snapshot_check               # 写快照并挪开窗口，验证启动恢复
cargo run -p airpane-platform-win --example watch_events -- --seconds 30 # 事件同步一致性
cargo run -p airpane-platform-win --example switch_check -- --rounds 200 # 前台切换成功率
cargo run -p airpane-platform-win --example idle_cpu -- --seconds 20     # 运行中 AirPane 的空闲 CPU 与窗口数
cargo run -p airpane-strip --example preview_strip -- --seconds 30       # 预览区 + 缩略图 + 拖动排序
```

## 文档

| 文件 | 内容 |
| --- | --- |
| [`docs/PRD.md`](docs/PRD.md) | 产品需求文档（2026-11-18 冻结的草案基线） |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | 决策记录，与 PRD 冲突时以此为准 |
| [`docs/test-report-M1.md`](docs/test-report-M1.md) | M1 的用例、结果、发现并修复的问题、已知缺口 |
| [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md) | 依赖清单与许可证核查 |
| [`docs/CHANGELOG.md`](docs/CHANGELOG.md) | 变更日志 |

## 参与

项目已停止开发，仍接受 issue 与 PR，但不承诺 review 时效，也不规划新功能（见 D-059）。

提交必须带 `Signed-off-by`（DCO）：

```powershell
git commit -s
```

每个 PR 需要 CI 通过（格式检查、Clippy、测试、构建）与 DCO 校验；提交前还要按 `AGENTS.md` 跑一遍 `code-review-expert` 并修完 P0/P1/P2。

## 许可证

[GPL-3.0-only](LICENSE)。
