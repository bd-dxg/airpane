# 变更日志

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循语义化版本。

## [Unreleased]

### 新增

- Cargo workspace 骨架：`airpane-core`、`airpane-platform-win`、`airpane-strip`、`airpane`
- 窗口快照数据结构（`Rect`、`ShowState`、`WindowSnapshot`、`SnapshotFile`）与 JSON 往返测试
- Win32 最小封装：`is_window` 句柄有效性查询
- GitHub Actions：格式检查、Clippy、测试、构建；PR 的 DCO 校验
- 中文 README、依赖清单（`docs/DEPENDENCIES.md`）、决策记录（`docs/DECISIONS.md`）
