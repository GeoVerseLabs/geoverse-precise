# geoverse-precise

Rust 编写、编译为 WebAssembly 的**椭球精度**客户端空间分析库，API 与 turf.js 对齐，用来替换 turf 中会产生失真、偏移的计算。

- **测量**：距离、方位角、目标点、中点、长度、面积、沿线取点、最近点、点到线距离 —— 全部基于 WGS84 椭球上的 Karney 大地线算法
- **缓冲区**：决定距离的每个顶点都在椭球上直接解算，任意范围都保持真实米制半径
- **叠加分析**：相交 / 合并 / 差集 / 异或，整数坐标内核（i_overlay），不会因浮点退化出错
- **坐标转换**：WGS84、CGCS2000、GCJ-02、BD-09、Web 墨卡托、UTM、CGCS2000 高斯-克吕格（3°/6° 带），高精度正反算
- **几何处理**：简化、凸包/凹包、中心点、包围盒裁剪、地理旋转平移缩放、线切分与相交、大圆线、要素级工具
- **空间关系**：完整 DE-9IM 矩阵与 contains / within / crosses / touches / overlaps / disjoint / equals
- **拓扑质量**：有效性检查与自动修复、面覆盖重叠与缝隙、线网络悬挂与伪节点、精度捕捉
- **格网与形状**：点/方/矩形/三角/六边形格网（米制，不随纬度拉伸）、椭圆、弧线、切线、掩膜、平滑、样条、成面、三角化
- **插值与等值线**：IDW 插值、等值线、等值面、TIN（Delaunay）、Voronoi、三角面内插值
- **聚类与点模式统计**：DBSCAN、k-means、最近邻分析、标准差椭圆、方向均值、空间权重、Moran's I、样方分析
- **路径**：绕障最短路径（米制格网 A* + 拉紧）
- **大数据量**：预处理几何 + 网格索引 + 批量接口（2 万点在面内判断从 5.5 秒降到 0.17 毫秒）

**turf 的 184 个导出全部同名覆盖**，逐函数对照见 [docs/TURF-COVERAGE.md](docs/TURF-COVERAGE.md)。
精度实测见 [docs/ACCURACY.md](docs/ACCURACY.md)，性能剖析与优化见 [docs/PERFORMANCE.md](docs/PERFORMANCE.md)，设计说明见 [docs/DESIGN.md](docs/DESIGN.md)。

## 快速开始

```ts
import * as gp from 'geoverse-precise';
await gp.init();

gp.distance([116.397, 39.909], [121.474, 31.230]);            // 1066.78 km（椭球）
gp.area(polygon);                                            // m²
gp.buffer(line, 500, { units: 'meters' });                   // 真实 500 m
gp.intersect(a, b);                                          // Feature | null
gp.transform(featureCollection, 'GCJ02', 'WGS84');           // 高德坐标 → WGS84
gp.transformCoords(float64Array, 'WGS84', 'EPSG:4549');      // 批量转 CGCS2000 3° 带
gp.gaussKrugerCrs(120.3);                                    // 'EPSG:4549'
```

大数据量场景用预处理几何，几何只解析一次：

```ts
const zone = gp.prepare(polygon, { units: 'meters' });
const inside = zone.containsMany(new Float64Array([116.4, 39.9, 121.5, 31.2]));  // Uint8Array
const near = zone.nearestMany(points);      // 每点 6 个数：x, y, dist, location, index, part
zone.free();
```

拓扑质量检查：

```ts
gp.validate(geojson);                                // { valid, issues[] }
gp.makeValid(geojson);                               // 修复后的要素 + geoverse-precise:fixes
gp.coverageIssues(parcels, { gapTolerance: 0.5 });   // 面之间的重叠与缝隙
gp.networkIssues(roads, { tolerance: 0.05, maxUndershoot: 2 });  // 悬挂线、伪节点、欠头
```

插值、聚类与统计：

```ts
gp.isolines(pointGrid, [100, 200, 300]);             // 等值线（按三角形线性插值，无鞍点歧义）
gp.isobands(pointGrid, [0, 100, 200]);               // 等值面，band 之间无缝无叠
gp.interpolate(samples, 1, { weight: 2 });           // IDW，搜索半径是真实米
gp.tin(samples); gp.voronoi(sites);                  // 在局部米制平面上三角化
gp.clustersDbscan(points, 2);                        // maxDistance 是大地线距离
gp.moranIndex(points, { threshold: 1.2 });           // 邻域半径是真实距离
gp.shortestPath(a, b, { obstacles, resolution: 1 }); // 绕障并拉紧
```

turf 的纯 JS 助手层（构造器、遍历、单位换算、`geojsonRbush`）也一并提供，且不需要 `init()`：

```ts
import { point, coordEach, convertLength, geojsonRbush } from 'geoverse-precise';
```

从 turf 迁移：函数名、参数顺序、默认单位（kilometers）、返回的 GeoJSON 形态都与 turf 一致。差异只有：

| 项 | turf | geoverse-precise |
|---|---|---|
| 初始化 | 无 | `await init()` 一次 |
| 地球模型 | 球（R = 6371008.8 m） | WGS84 椭球 |
| 输入坐标系 | 仅 WGS84 | 任意函数可传 `crs`，例如 `{ crs: 'GCJ02' }` |
| 长边解释 | 经纬度直线 | 默认同 turf（`edges: 'planar'`），可选 `'geodesic'` |
| 单位 | 支持 radians / degrees | 仅长度单位（角度单位在椭球上无意义） |
| 格网 / 容差 / 半径 | 换算成度 | 真实米制（这是格网与统计结果差异的来源） |

### 通用选项

| 选项 | 适用 | 说明 |
|---|---|---|
| `units` | 长度相关 | 默认 `kilometers` |
| `crs` | 全部测量 / 构造函数 | 输入坐标系；结果以同一坐标系返回 |
| `edges` | length, area, along, nearestPointOnLine, pointToLineDistance, buffer, overlay | `planar`（默认，GeoJSON 标准）或 `geodesic` |
| `tolerance` | buffer, overlay | 工作平面中弦与真实曲线的最大偏差（米），默认 0.01 |
| `method` | buffer | `geodesic`（默认，精确）或 `projected`（局部投影，快速，适合几十公里内） |
| `steps` | buffer / circle | buffer 为每 1/4 圆的段数（默认 16）；circle 为整圆段数（默认 64） |

### 函数一览

| 类别 | 函数 |
|---|---|
| 测量 | `distance` `bearing` `destination` `midpoint` `length` `area` `along` `nearestPointOnLine` `pointToLineDistance` `circle` |
| 缓冲与叠加 | `buffer` `intersect` `union` `difference` `xor` `unionAll` |
| 几何处理 | `simplify` `convexHull` `concaveHull` `centroid` `centerOfMass` `pointOnFeature` `bbox` `bboxPolygon` `bboxClip` `transformRotate` `transformTranslate` `transformScale` `lineSlice` `lineSliceAlong` `lineChunk` `lineIntersect` `greatCircle` `sector` `nearestPoint` `pointsWithinPolygon` |
| 要素工具 | `flatten` `explode` `polygonToLine` `lineToPolygon` `rewind` `cleanCoords` `truncate` `dissolve` `center` `envelope` `square` `combine` `sample` `tag` `collect` `flip` |
| 恒向线 | `rhumbDistance` `rhumbBearing` `rhumbDestination` |
| 线工具 | `lineSegment` `lineSplit` `lineOffset` `lineOverlap` `nearestPointToLine` `pointToPolygonDistance` `angle` `kinks` `unkinkPolygon` |
| 格网与形状 | `pointGrid` `squareGrid` `rectangleGrid` `triangleGrid` `hexGrid` `ellipse` `lineArc` `polygonSmooth` `polygonTangents` `mask` `bezierSpline` `polygonize` `tesselate` `centerMean` `centerMedian` |
| 插值与等值线 | `interpolate` `isolines` `isobands` `tin` `voronoi` `planepoint` |
| 聚类与统计 | `clustersDbscan` `clustersKmeans` `nearestNeighborAnalysis` `standardDeviationalEllipse` `directionalMean` `distanceWeight` `moranIndex` `quadratAnalysis` |
| 路径 | `shortestPath` |
| 纯 TS 助手 | `point` `lineString` `polygon` 等构造器、`coordEach` / `segmentEach` 等遍历、`convertLength` / `convertArea` 等换算、`geojsonRbush`、`meta` / `helpers` / `invariant` / `projection` / `random` / `clusters` 命名空间 |
| 空间关系 | `relate` `relatePattern` `booleanRelation` `booleanContains` `booleanWithin` `booleanCrosses` `booleanTouches` `booleanOverlap` `booleanDisjoint` `booleanEqual` `booleanPointInPolygon` `booleanIntersects` `booleanPointOnLine` |
| 拓扑质量 | `validate` `makeValid` `coverageIssues` `networkIssues` `snapRound` `snapTo` |
| 批量与索引 | `prepare` / `Prepared`（`contains` `containsMany` `nearest` `nearestMany` `distance` `distanceMany` `within`）`distanceBatch` `distanceToBatch` `destinationBatch` `transformCoords` |
| 坐标系 | `transform` `convert` `gcj02ToWgs84` 等六个快捷方法 `gaussKrugerCrs` `utmCrs` `normalizeCrs` |

### 支持的坐标系

`WGS84` / `EPSG:4326`，`CGCS2000` / `EPSG:4490`，`GCJ02`，`BD09`，`EPSG:3857`，
`EPSG:32601–32660` / `EPSG:32701–32760`（UTM），`EPSG:4491–4554`（CGCS2000 高斯-克吕格），
以及自定义横轴墨卡托 `{ proj: 'tmerc', lon0: 114, x0: 500000, ellps: 'CGCS2000' }`。

## 目录

```
crates/core          纯 Rust 核心（可脱离 WASM 使用）
crates/wasm          wasm-bindgen 导出层
packages/geoverse-precise  TypeScript 封装（npm 包）
examples/web-demo    MapLibre 对比页：turf 与 geoverse-precise 缓冲区叠加显示，可切换高德底图
bench                精度 / 速度对比脚本、turf 逐函数对照脚本、剖析脚本与基准数据
docs                 设计说明、精度报告、性能剖析、turf 覆盖对照表
```

## 构建

依赖：Rust ≥ 1.88、`wasm32-unknown-unknown` target、与 `crates/wasm/Cargo.toml` 同版本的 `wasm-bindgen-cli`、Node ≥ 18；可选 binaryen ≥ 116（`wasm-opt`；更老的版本会把 externref 表重编号而不修正导出，构建脚本会自动跳过）。

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128

cargo test -p geoverse-precise-core          # Rust 单测 + GeographicLib / PROJ 对照
./scripts/build-wasm.sh                # 生成 packages/geoverse-precise/wasm
cd packages/geoverse-precise && npm install && npm run build && npm test

cd ../../examples/web-demo && npm install && npm run dev   # 对比演示页
```

重新生成基准数据与精度报告：

```bash
pip install geographiclib pyproj numpy
python bench/gen_fixtures.py
cd bench && npm install && node accuracy.mjs && python verify_buffer.py
```

turf 逐函数行为对照（输出 `bench/out/parity.json`）：

```bash
cd bench && node turf-parity.mjs
```

## 版本管理

- 版本号以 `Cargo.toml`（`[workspace.package] version`）与 `packages/geoverse-precise/package.json` 为准，两处保持一致；
  发布时打附注 tag `vX.Y.Z`，变更记录写进 [CHANGELOG.md](CHANGELOG.md)。
  文档里的 v1 / v1.1 / v1.2 是历史里程碑名称，对应 0.1.0 / 0.2.0 / 0.3.0。
- 分支：`main` 保持可构建、测试全绿；新功能在 `feat/<名称>`、修复在 `fix/<名称>` 上开发后合入。
- 提交信息采用 `类型: 说明` 格式（`feat` / `fix` / `perf` / `docs` / `test` / `refactor` / `style` / `chore`）。
- **构建产物不入库**：`packages/geoverse-precise/dist/`、`packages/geoverse-precise/wasm/`、`examples/web-demo/dist/`
  都由源码生成（见上文「构建」）；v0.3.0 的预编译产物保存在交付包 `geoprecise-v0.3.0_1.zip` 中。
- 提交前检查：

  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace
  cargo test -p geoverse-precise-core
  cd packages/geoverse-precise && npm run build && npm test
  ```

- 纯格式化提交登记在 `.git-blame-ignore-revs`，执行一次 `git config blame.ignoreRevsFile .git-blame-ignore-revs` 即可让 `git blame` 跳过它们。

## 许可

MIT OR Apache-2.0
