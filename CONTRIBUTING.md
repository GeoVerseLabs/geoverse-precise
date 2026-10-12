# 参与开发

## 环境

| 工具 | 版本 | 用途 |
|---|---|---|
| Rust | ≥ 1.88（`rust-toolchain` 用 stable 即可） | 核心库与 wasm 导出 |
| `wasm32-unknown-unknown` target | — | `rustup target add wasm32-unknown-unknown` |
| `wasm-bindgen-cli` | **0.2.128**，必须与 `crates/wasm/Cargo.toml` 一致 | `cargo install wasm-bindgen-cli --version 0.2.128 --locked` |
| Node.js | ≥ 20（CI 用 22，并在 20 / 24 上复测） | TS 封装、测试、基准 |
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

`main` 只通过 PR 更新。发版 = 合并一个提升了版本号的 PR，标签与 Release 由工作流自动创建。

1. 新建分支 `release/vX.Y.Z`，在其中：
   - 把 `CHANGELOG.md` 的 `[Unreleased]` 整理为 `## [X.Y.Z] - YYYY-MM-DD`，并新建空的 `[Unreleased]`；
   - 同步版本号：`Cargo.toml`（`[workspace.package] version`）与 `packages/geoverse-precise/package.json`，
     然后执行 `cargo check` 与 `cd packages/geoverse-precise && npm install --package-lock-only` 更新锁文件；
   - 提交 `chore(release): vX.Y.Z`，推送并发 PR。
2. **演练**（可选，推荐）：Actions → Release → Run workflow，分支选 `release/vX.Y.Z`，填入 `X.Y.Z`。
   演练会做完版本与 CHANGELOG 校验、编译、测试、打包、`publint`，产物（`.tgz`、`SHA256SUMS.txt`、发布说明）
   作为 artifact 供下载检查，不创建标签和 Release。
3. CI 全绿后合并 PR。`release.yml` 在 `main` 上发现 `vX.Y.Z` 标签不存在，于是自动：
   - 重复第 2 步的全部校验与构建，为 `.tgz` 生成构建来源证明（attestation）；
   - 在合并后的提交上创建标签 `vX.Y.Z` 与 GitHub Release，附 `.tgz` 与 `SHA256SUMS.txt`，说明取自 CHANGELOG 对应段落；
   - 已开启 npm 发布时（见下文），把同一个 `.tgz` 发布到 npm；
     版本号带 `-`（如 `0.5.0-rc.1`）时标记为预发布；
   - 若已开启 GitHub Pages（Settings → Pages → Source 选 GitHub Actions），部署演示页；未开启则跳过并给出提示。
4. 合并后同步本地：`git switch main && git pull --ff-only && git fetch --tags`。

版本号已有标签时，合并到 `main` 不会触发发版，所以日常 PR 不受影响。
也可以照旧手动推送附注标签发版（`git tag -a vX.Y.Z -m "…" && git push origin vX.Y.Z`），流程相同。

### 发布到 npm

npm 包名为 `@geoverse/geoverse-precise`，归属 npm 组织 `geoverse`。发版时 `release.yml` 的 `npm` 任务以**可信发布**
（GitHub Actions OIDC，无需 token）把 Release 中的同一个 `.tgz` 发布到 npm，并自动生成 provenance。
该任务只在仓库变量 `NPM_PUBLISH` 为 `true` 时执行。

一次性准备（npm 要求包先存在，才能配置可信发布）：

1. **首次手动发布**：从 GitHub Release 下载 `geoverse-geoverse-precise-X.Y.Z.tgz`，用 `geoverse` 组织中有发布权限、
   已开启两步验证的账号执行 `npm login`，然后 `npm publish ./geoverse-geoverse-precise-X.Y.Z.tgz --access public`。
2. **配置可信发布**：npmjs.com → 包 `@geoverse/geoverse-precise` → Settings → Trusted Publisher → GitHub Actions：
   Organization or user `GeoVerseLabs`，Repository `geoverse-precise`，Workflow filename `release.yml`，Environment 留空。
3. **打开开关**：GitHub 仓库 Settings → Secrets and variables → Actions → Variables，新建 `NPM_PUBLISH` = `true`。
4. 可信发布配置在首次成功发布前只保留 48 小时，过期后需重新创建。所以第 2、3 步最好放在下一次发版 PR 合并前进行；
   首次通过 CI 发布成功后配置即长期有效，之后可在包设置中开启 “Require two-factor authentication and disallow tokens”。

crates.io 目前不自动发布；发布 `geoverse-precise-core` 前注意需带上许可证文本（目前只在仓库根目录）。

## 升级 wasm-bindgen

`wasm-bindgen` 的库版本与 CLI 版本必须完全一致，因此 Dependabot 已忽略它，需手动同步修改两处：

- `crates/wasm/Cargo.toml` 中的 `wasm-bindgen = "=x.y.z"`
- `.github/actions/build-wasm/action.yml` 中 `wasm-bindgen-version` 的默认值（CI / Release / Demo 共用；两处不一致时构建会直接报错）

另外更新本文与 README「构建」一节中的安装命令。
