# 依赖清单

引入任何依赖前先在此登记，并确认许可证与 `GPL-3.0-only` 兼容（PRD 11.3 第 7 条）。

版本策略：`Cargo.toml` 里写最小版本，实际锁定以仓库中的 `Cargo.lock` 为准；升级依赖必须同步更新本文件的版本号。

## 直接依赖

| 依赖 | 版本 | 许可证 | 用途 | 引入任务 |
| --- | --- | --- | --- | --- |
| `windows` | 0.62.2 | MIT OR Apache-2.0 | Win32 / DWM / DXGI / DirectComposition 绑定 | T0.1 || `serde` | 1.0.229（含 `serde_derive`） | MIT OR Apache-2.0 | 窗口快照与配置的序列化 | T0.1 |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | 窗口快照与配置的 JSON 读写：`airpane-core` 的 dev-dependency，`airpane-platform-win` 的直接依赖（T1.7 起） | T0.1 |

## 传递依赖

由 `cargo metadata` 导出，全部为 MIT / Apache-2.0 类许可，与 GPL-3.0 兼容。

| 依赖 | 版本 | 许可证 |
| --- | --- | --- |
| `itoa` | 1.0.19 | MIT OR Apache-2.0 |
| `memchr` | 2.8.3 | Unlicense OR MIT |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 |
| `quote` | 1.0.47 | MIT OR Apache-2.0 |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 |
| `syn` | 2.0.119 / 3.0.10 | MIT OR Apache-2.0 |
| `unicode-ident` | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| `windows-collections` | 0.3.2 | MIT OR Apache-2.0 |
| `windows-core` | 0.62.2 | MIT OR Apache-2.0 |
| `windows-future` | 0.3.2 | MIT OR Apache-2.0 |
| `windows-implement` | 0.60.2 | MIT OR Apache-2.0 |
| `windows-interface` | 0.59.3 | MIT OR Apache-2.0 |
| `windows-link` | 0.2.1 | MIT OR Apache-2.0 |
| `windows-numerics` | 0.3.1 | MIT OR Apache-2.0 |
| `windows-result` | 0.4.1 | MIT OR Apache-2.0 |
| `windows-strings` | 0.5.1 | MIT OR Apache-2.0 |
| `windows-threading` | 0.2.1 | MIT OR Apache-2.0 |
| `zmij` | 1.0.23 | MIT |

## CI 使用的 Action

| Action | 许可证 | 用途 |
| --- | --- | --- |
| `actions/checkout` | MIT | 检出代码，DCO 检查需要完整历史（`fetch-depth: 0`） |
| `Swatinem/rust-cache` | MIT | 缓存 Cargo 编译产物，缩短 Windows runner 的构建时间 |

## 待办

- Slint（GPLv3 使用方式已核实）在 M3 引入时登记，同时记录与 egui 的对比结论（D-017）。
- 后续新增的 GUI、更新器、日志依赖逐个核查后追加。
