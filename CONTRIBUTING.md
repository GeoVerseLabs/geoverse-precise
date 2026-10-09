# 参与开发

## 环境

| 工具 | 版本 | 用途 |
|---|---|---|
| Rust | ≥ 1.88（`rust-toolchain` 用 stable 即可） | 核心库与 wasm 导出 |
| `wasm32-unknown-unknown` target | — | `rustup target add wasm32-unknown-unknown` |
| `wasm-bindgen-cli` | **0.2.128**，必须与 `crates/wasm/Cargo.toml` 一致 | `cargo install wasm-bindgen-cli --version 0.2.128 --locked` |
| Node.js | ≥ 18（CI 用 22） | TS 封装、测试、基准 |
| binaryen（可选） | ≥ 116 | `wasm-opt` 体积优化；更老的版本构建脚本会自动跳过 |
| Python（可选） | 3.10+，`geographiclib pyproj numpy` | 重新生成基准数据、复核缓冲区 |

Windows 下请在 **Git Bash** 中运行 `scripts/build-wasm.sh`。仓库通过 `.gitattributes` 统一 LF 换行，不要改成 CRLF。

## 日常流程

```bash
git switch -c feat/<名称>          # 修复用 fix/<名称>

# 修改 Rust 核心后
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p geoverse-precise-core

# 修改了 wasm 导出或 TS 封装后
./scripts/build-wasm.sh
cd packages/geoverse-precise && npm run build && npm test
```

本地通过后推送分支、发 PR 到 `main`，CI（`.github/workflows/ci.yml`）全绿后合并。

## 代码约定

- **core 与绑定分离**：算法只写在 `crates/core`；`crates/wasm` 只做一比一包装；TS 层只做参数规整与 GeoJSON 进出。
- **精度改动要有对照**：任何会改变计算结果的修改，都要附上与 GeographicLib / PROJ 的对照（`bench/accuracy.mjs`、`crates/core/tests/reference.rs`），必要时更新 `docs/ACCURACY.md`。
- **与 turf 的差异要写明**：新增或调整 turf 同名函数时，同步更新 `bench/turf-parity.mjs` 与 `docs/TURF-COVERAGE.md`。
- **新增 API 要有测试**：Rust 单测放在模块内 `#[cfg(test)]`，JS 行为测试放在 `packages/geoverse-precise/test/`。

## 提交信息

格式：`类型: 说明`，说明用中文或英文均可。

| 类型 | 用途 |
|---|---|
| `feat` | 新功能 |
| `fix` | 缺陷修复 |
| `perf` | 性能优化 |
| `refactor` | 不改变行为的重构 |
| `docs` / `test` / `style` / `chore` / `ci` | 文档、测试、格式、杂项、CI |

不兼容变更在类型后加 `!`（如 `refactor!:`），并在正文写 `BREAKING CHANGE: …`。
纯格式化的大提交请把哈希登记进 `.git-blame-ignore-revs`。

## 发布

1. 在 `CHANGELOG.md` 中把 `[Unreleased]` 改为 `[X.Y.Z] - 日期`，并新建空的 `[Unreleased]`；
2. 同步修改版本号：`Cargo.toml`（`[workspace.package] version`）与 `packages/geoverse-precise/package.json`；
3. 提交：`chore: release vX.Y.Z`；
4. 打附注标签并推送：

   ```bash
   git tag -a vX.Y.Z -m "geoverse-precise vX.Y.Z"
   git push origin main vX.Y.Z
   ```

5. `.github/workflows/release.yml` 会校验标签与两处版本号一致，重新编译 wasm、跑测试，
   然后创建 GitHub Release，附上 npm 包（`.tgz`），说明取自 CHANGELOG 对应段落。

目前不自动发布到 npm / crates.io。需要时在 release 之后手动执行 `npm publish` / `cargo publish`。

## 升级 wasm-bindgen

`wasm-bindgen` 的库版本与 CLI 版本必须完全一致，因此 Dependabot 已忽略它，需手动同步修改三处：

- `crates/wasm/Cargo.toml` 中的 `wasm-bindgen = "=x.y.z"`
- `.github/workflows/ci.yml` 与 `release.yml` 中的 `WASM_BINDGEN_VERSION`
- 本文与 README「构建」一节中的安装命令
