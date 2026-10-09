# 更新日志

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [SemVer](https://semver.org/lang/zh-CN/)。

> **版本号对照**：设计文档里的里程碑编号（v1 / v1.1 / v1.2）与发布版本号的对应关系：
>
> | 里程碑（docs 中的叫法） | 发布版本（Cargo / npm） |
> |---|---|
> | v1 | 0.1.0 |
> | v1.1 | 0.2.0 |
> | v1.2 | **0.3.0** |
>
> 今后统一使用发布版本号；docs 中的 v1.x 仅作为历史里程碑名称保留。
>
> 0.1.0 与 0.2.0 未单独存档源码，仓库历史从 0.3.0 开始（tag `v0.3.0`）；
> 下面两个版本的条目根据 `docs/DESIGN.md`、`docs/PERFORMANCE.md` 与测试文件整理，日期按文件时间推断。

## [Unreleased]

### ⚠ 不兼容变更：项目更名为 geoverse-precise
仓库迁至 [GeoVerseLabs/geoverse-precise](https://github.com/GeoVerseLabs/geoverse-precise)，所有包名随之更改：

| 项 | 0.3.0 及以前 | 现在 |
|---|---|---|
| npm 包 | `geoprecise` | `geoverse-precise` |
| Rust crate | `geoprecise-core` / `geoprecise-wasm` | `geoverse-precise-core` / `geoverse-precise-wasm` |
| Rust 路径 | `geoprecise_core::…` | `geoverse_precise_core::…` |
| wasm 产物 | `geoprecise_wasm*.{js,wasm}` | `geoverse_precise_wasm*.{js,wasm}` |
| 源码目录 | `packages/geoprecise` | `packages/geoverse-precise` |
| `makeValid` 结果属性 | `geoprecise:fixes` | `geoverse-precise:fixes` |
| `snapTo` 结果属性 | `geoprecise:moved` | `geoverse-precise:moved` |

迁移：`import … from 'geoprecise'` 改为 `from 'geoverse-precise'`；读取上述两个结果属性的代码同步改键名。
函数名、参数与计算结果均无变化。`bench/out/` 下的历史结果文件保留 0.3.0 运行时的原始键名。

### 变更
- 全部 Rust 源码按 `rustfmt.toml` 统一格式化（纯格式，无逻辑改动；`cargo test` 结果不变）。
- 新增本更新日志；README 增加「版本管理」一节。
- Cargo 与 npm 清单补充 `repository` / `homepage` / `bugs` 等元数据。

### 计划（来自 DESIGN.md §8）
- `Prepared` 复用 area / length / buffer；二进制几何通道；`precision: 'fast'` 快速测量；Web Worker 封装；cargo feature 瘦身。

## [0.3.0] - 2026-09-30 · 里程碑 v1.2

turf 覆盖层：`@turf/turf` 的 **184 个导出全部同名实现**，可整包替换。

> 自此版本纳入 git 管理。tag `v0.3.0` 的源码与交付包 `geoprecise-v0.3.0_1.zip` 一致，
> 仅增加 `.gitattributes` / `.editorconfig` 并完善 `.gitignore`；构建产物（`dist/`、`wasm/`、演示页 `dist/`）不入库，
> 预编译产物以交付包为准。

### 新增
- 恒向线：`rhumbDistance` / `rhumbBearing` / `rhumbDestination`（子午线弧长 Gauss–Legendre 积分 + Newton 反算）。
- 线工具：`lineSegment` `lineSplit` `lineOffset` `lineOverlap` `nearestPointToLine` `pointToPolygonDistance` `angle` `kinks` `unkinkPolygon`。
- 米制格网：`pointGrid` `squareGrid` `rectangleGrid` `triangleGrid` `hexGrid`（局部横轴墨卡托平面布点，不随纬度拉伸）。
- 形状构造：`ellipse` `lineArc` `polygonSmooth` `polygonTangents` `mask` `bezierSpline` `polygonize` `tesselate` `centerMean` `centerMedian` 等。
- 插值与等值线：`interpolate`（IDW）`isolines` `isobands` `tin` `voronoi` `planepoint`；等值线按格网三角形求解，无鞍点歧义。
- 聚类与统计：`clustersDbscan` `clustersKmeans`（确定性 k-means++ 播种）`nearestNeighborAnalysis` `standardDeviationalEllipse` `directionalMean` `distanceWeight` `moranIndex` `quadratAnalysis`。
- 路径：`shortestPath`（米制格网 A* + string pulling）。
- 纯 TS 助手层（无需 `init()`）：构造器、`coordEach` 等遍历、单位换算、`geojsonRbush`，以及 `meta` / `helpers` / `invariant` / `projection` / `random` / `clusters` 命名空间。
- `bench/turf-parity.mjs`：与 turf 逐函数行为对照，输出 `bench/out/parity.json` / `parity.txt`；对照结论见 `docs/TURF-COVERAGE.md`。

### 修复
- `centroid`：不再把环的闭合点计入顶点均值（此前在正方形上会偏 1/4 边长），与 turf 一致。
- TIN：在输出空间（经纬度）判定并剔除退化三角形，三角形数量与 turf 一致。

### 已知限制
- Voronoi 共享边写成经纬度弦后有约 1e-4 面积占比的发丝缝；`lineOffset` 凹角不做尖角延伸。完整列表见 `docs/DESIGN.md` §7。

## [0.2.0] - 2026-09-29（推断） · 里程碑 v1.1

### 新增
- 预处理几何 `prepare()` / `Prepared` + 经纬度网格索引；批量接口 `containsMany` `nearestMany` `distanceMany` `within`、`distanceBatch` `distanceToBatch` `destinationBatch`。
- 几何处理：`simplify` `convexHull` `concaveHull` `centroid` `centerOfMass` `pointOnFeature` `bbox*` `transformRotate/Translate/Scale` `lineSlice*` `lineChunk` `lineIntersect` `greatCircle` `sector` 及要素工具。
- DE-9IM：`relate` `relatePattern` 与 `booleanContains / Within / Crosses / Touches / Overlap / Disjoint / Equal / Relation / PointOnLine`。
- 拓扑质量：`validate` `makeValid` `coverageIssues` `networkIssues` `snapRound` `snapTo`。

### 性能
- 2 万点在面内（500 顶点面）：5456 ms → 0.17 ms（批量）。
- `pointToLineDistance`（280 段线）：10.4 ms → 0.63 ms；横轴墨卡托改 Clenshaw 递推；平面边长度/面积改闭式积分。
- 剖析过程与数据见 `docs/PERFORMANCE.md`。

## [0.1.0] - 2026-09-16 · 里程碑 v1

首个版本：Rust 核心 + wasm-bindgen 导出 + TypeScript 封装，API 与 turf 对齐。

### 新增
- 测量：`distance` `bearing` `destination` `midpoint` `length` `area` `along` `nearestPointOnLine` `pointToLineDistance` `circle`，全部基于 WGS84 椭球 Karney 大地线算法。
- 缓冲区：大地线缓冲（默认，顶点在椭球上直接解算）与投影缓冲（快速）。
- 叠加：`intersect` `union` `difference` `xor` `unionAll`（i_overlay 整数内核，局部横轴墨卡托工作平面）。
- 坐标转换：WGS84 / CGCS2000 / GCJ-02 / BD-09 / Web 墨卡托 / UTM / CGCS2000 高斯-克吕格（3°/6° 带），GCJ-02 与 BD-09 逆算迭代求解。
- 通用选项 `units` `crs` `edges` `tolerance`。
- 与 GeographicLib / PROJ 的对照测试与精度基准（`bench/`）、MapLibre 对比演示页（`examples/web-demo`）。
