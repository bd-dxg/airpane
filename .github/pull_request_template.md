## 变更内容

<!-- 一句话说明这个 PR 做了什么，以及为什么这么做 -->

## 对应任务

<!-- PRD 中的任务编号，例如 T0.1、T0.2 -->

## 验证

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] 手动验证（涉及窗口操作的改动必填，写清测试的应用与结果）

## 检查项

- [ ] 提交均带 `Signed-off-by`（`git commit -s`）
- [ ] 新增依赖已登记在 `docs/DEPENDENCIES.md`
- [ ] 与 PRD 不一致的取舍已记录在 `docs/DECISIONS.md`
- [ ] 没有向其他进程注入代码、没有安装键盘钩子（PRD 11.3）
