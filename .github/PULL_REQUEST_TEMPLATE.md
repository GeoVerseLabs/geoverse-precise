## 改了什么

<!-- 一两句话说明改动与动机；关联 issue 写 Closes #123 -->

## 类型

- [ ] feat 新功能
- [ ] fix 缺陷修复
- [ ] perf 性能
- [ ] docs / test / refactor / chore
- [ ] **不兼容变更**（API、默认值、返回结构或计算结果有变化）

## 检查

- [ ] `cargo fmt --all -- --check` 与 `cargo clippy --workspace --all-targets -- -D warnings` 通过
- [ ] `cargo test -p geoverse-precise-core` 通过
- [ ] 改动涉及 wasm 导出或 TS 封装时：`./scripts/build-wasm.sh` 后 `npm run build && npm test` 通过
- [ ] 改动影响计算结果时：附上与 GeographicLib / PROJ / turf 的对照数据，必要时更新 `docs/ACCURACY.md`
- [ ] 已在 `CHANGELOG.md` 的 `[Unreleased]` 下记录
