# AirPane（轻窗）

Windows 上的「台前调度」式窗口管理：当前窗口待在屏幕中间，其余窗口以缩略图常驻一侧，点击即切换。窗口位置固定可预期，减少 Alt+Tab 里逐个找窗口的成本。

> **状态：M0 技术原型阶段。** 现在只在验证核心可行性（窗口枚举、事件同步、预览区、前台切换、动画路线），没有可用界面，行为与接口随时可能大幅变化，请勿日常使用。

## 规划中的能力

- **台前调度模式**：屏幕分为预览区与舞台区，非活跃窗口按最近使用顺序排列在预览区，点击即切到舞台区。
- **与原位置还原**：切回普通桌面时，所有被本程序移动或缩放的窗口恢复到原位置、大小与最大化状态。
- **游戏与全屏零冲突**：命中进程名单或检测到全屏时自动暂停，条件解除后自动恢复。
- **窗口分组**：把最多 3 个窗口编成一组，在舞台区并排显示。
- **不注入、不挂钩**：只读取窗口信息，不向任何进程注入代码，不安装键盘钩子。

## 当前进度

M0：窗口枚举与过滤 → WinEvent 事件同步 → 可靠的前台切换 → 预览区与 DWM 缩略图 → 动画路线验证。

路线与阶段见 [`docs/PRD.md`](docs/PRD.md)（第 9、11 节）；与 PRD 原文不一致的决策见 [`docs/DECISIONS.md`](docs/DECISIONS.md)。

## 构建

需要 Windows 10 22H2 / Windows 11 与 Rust stable（`x86_64-pc-windows-msvc`）。

```powershell
cargo build --workspace
cargo run --bin airpane
```

M0 阶段的主程序只打印版本与快照格式版本，还没有界面。

## 文档

| 文件 | 内容 |
| --- | --- |
| [`docs/PRD.md`](docs/PRD.md) | 产品需求文档（2026-11-18 冻结的草案基线） |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | 决策记录，与 PRD 冲突时以此为准 |
| [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md) | 依赖清单与许可证核查 |
| [`docs/CHANGELOG.md`](docs/CHANGELOG.md) | 变更日志 |

## 参与

接受外部 PR，提交必须带 `Signed-off-by`（DCO）：

```powershell
git commit -s
```

每个 PR 需要 CI 通过（格式检查、Clippy、测试、构建），并附带 DCO 校验。

## 许可证

[GPL-3.0-only](LICENSE)。
