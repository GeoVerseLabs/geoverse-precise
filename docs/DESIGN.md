# geoprecise v1 设计说明

> 目标：用 Rust 编译成 WebAssembly，在浏览器端提供**椭球精度**的空间分析，替换 turf.js 中会产生失真、偏移的那部分能力。
> v1 范围：测量、缓冲区、叠加分析、坐标转换。API 与 turf 对齐，可以逐个函数替换。
> 实测数据见 [ACCURACY.md](ACCURACY.md)。

---

## 1. 问题分析：turf.js 的误差来自哪里

基于 `@turf/turf@7.4.0` 源码核对，并用 GeographicLib / PROJ 基准数据实测：

| 现象 | turf 的实现 | 实测误差 |
|---|---|---|
| 距离、目标点不准 | haversine 与球面公式，地球半径固定 6371008.8 m | 距离相对误差最高 **0.50%**（国内 0.44%）；`destination` 在 5000 km 内最大偏 16 km |
| 面积不准 | 球面多边形公式，半径同上 | 小多边形 0.19%~0.41% |
| 缓冲区变形 | 先用 d3 球面等距方位投影（中心取 bbox 中心），再用 JSTS 平面缓冲，最后反投影 | 1 km 点缓冲偏 2.5 m；1100 km 线路缓冲偏 6.3 m；550 km 面缓冲偏 16.8 m |
| 叠加结果偏移 | 叠加本身在经纬度平面上做，几何上没有问题；偏移主要来自**输入坐标系不一致**（WGS84 / GCJ-02 / BD-09 混叠） | 几十到几百米 |
| 国内坐标系 | turf 不支持，通常配合 coordtransform / gcoord | coordtransform 逆算残差最大 4.5 m，gcoord 最大 13.5 cm |

结论：问题分三类——**地球模型**（正球体 → 应换成 WGS84 椭球）、**平面化方式**（单一投影中心 → 顶点应在椭球上直接求出）、**坐标系管理**（国内坐标系与投影坐标需要高精度互转）。

> 补充：proj4js 的横轴墨卡托精度与 PROJ 一致（纳米级），引入 geoprecise 的投影能力主要是为了统一接口和批量性能（20 万点快约 17 倍），而不是修正 proj4js 的精度。

---

## 2. 总体架构

```
┌──────────────────────── 浏览器 / Node ─────────────────────────┐
│  packages/geoprecise  TypeScript，turf 风格 API，GeoJSON 进出    │
│        │  JSON 字符串 / Float64Array                            │
│  crates/wasm          wasm-bindgen 导出层（很薄）               │
│        │                                                        │
│  crates/core          纯 Rust，无 JS 依赖                       │
│   ├─ api          JSON 进出的门面：单位、crs、edges 等参数处理   │
│   ├─ geodesic / gnomonic   Karney 大地线、椭球心射投影          │
│   ├─ measure      距离、方位、长度、面积、沿线、最近点           │
│   ├─ crs          WGS84 / CGCS2000 / GCJ-02 / BD-09 /           │
│   │               Web 墨卡托 / UTM / CGCS2000 高斯-克吕格        │
│   ├─ densify      边的解释方式、自适应加密的误差上界             │
│   ├─ local        局部横轴墨卡托工作平面                         │
│   ├─ buffer       大地线缓冲（默认）+ 投影缓冲（快速）           │
│   ├─ overlay      相交 / 合并 / 差集 / 异或（i_overlay 整数内核） │
│   ├─ index        预处理几何 + 网格索引（批量点查询）            │
│   ├─ ops          简化、凸/凹包、中心、变换、线切分、结构工具     │
│   ├─ predicates   DE-9IM 矩阵与命名关系判断                      │
│   ├─ validate     几何有效性检查与自动修复                       │
│   └─ topology     面覆盖质量、线网络拓扑、精度捕捉               │
└─────────────────────────────────────────────────────────────────┘
```

设计原则：

1. **core 与绑定分离**。`api` 模块是字符串进出的门面，WASM 层一比一包装；以后接 Tauri、Node N-API 或服务端都复用它。
2. **数据交换走 JSON 字符串**。大几何逐值跨边界更慢，`JSON.stringify` + `serde_json` 更快；批量坐标转换走 `Float64Array`，原地修改。
3. **默认值与 turf 一致**：长度单位 `kilometers`，面积 m²，边按经纬度直线解释，返回的 GeoJSON 形态一致（空结果返回 `null` / `undefined`）。

依赖：`geographiclib-rs`（Karney 算法）、`geo` 0.33（i_overlay 布尔运算、平面缓冲、谓词）、`geojson` 1.0。

---

## 3. 精度策略

### 3.1 地球模型

- 所有测量都在 WGS84 椭球上用 **Karney 大地线算法**，任意距离收敛，含近对跖点。
- 面积使用 Karney 椭球多边形面积（带高精度累加）。
- CGCS2000 经纬度与 WGS84 视为同一框架（差异在分米级以内）；投影计算使用各自的椭球参数（CGCS2000 `f = 1/298.257222101`）。

### 3.2 边的解释（`edges`）

同两个顶点之间的边有两种合理解释，长边上差别很大（30°N 一条 1° 东西向边，两者相差约 106 m）：

| 取值 | 含义 | 使用场景 |
|---|---|---|
| `planar`（默认） | 经纬度直线。GeoJSON 标准（RFC 7946 §3.1.1）与 turf 的约定；网页地图上东西、南北向的边就是这样画的 | 行政区划、手绘围栏、网格 |
| `geodesic` | 椭球上的最短路径 | 航线、测量数据 |

内部实现统一按大地线计算，`planar` 通过加密逼近：一条方位角为 α 的经纬度直线，其大地曲率约为 `sinα·(1+cos²α)·tanφ / R`，弦高为 `L²κ/8`。据此解析地算出需要分几段（带 25% 余量，使用 WGS84 最小曲率半径），不需要额外的大地线计算。随机测试验证了误差上界。

### 3.3 坐标转换

| 坐标系 | 标识 | 算法 |
|---|---|---|
| WGS84 | `WGS84`, `EPSG:4326` | 转换枢纽 |
| CGCS2000 经纬度 | `CGCS2000`, `EPSG:4490` | 与 WGS84 同框架 |
| GCJ-02 | `GCJ02` | 公开加偏算法；**逆算迭代求解**到 1e-10° 以内 |
| BD-09 | `BD09` | 公开算法；逆算同样迭代 |
| Web 墨卡托 | `EPSG:3857` | 球面墨卡托（R = 6378137） |
| UTM | `EPSG:326xx / 327xx` | 横轴墨卡托 |
| CGCS2000 高斯-克吕格 | `EPSG:4491–4554` | 3° / 6° 带，带号前缀与不带前缀两种；`gaussKrugerCrs(lon)` 按经度自动选带 |
| 自定义横轴墨卡托 | `{proj:'tmerc', lon0, lat0, k0, x0, y0, ellps}` | 同上 |

横轴墨卡托使用 **Krüger 六阶 n 级数**（Karney 2011），距中央经线 3900 km 以内误差约 5 nm；与 PROJ 对照最大差 22 nm。
所有转换以 WGS84 经纬度为枢纽：`源 → WGS84 → 目标`（GCJ-02 ↔ BD-09 直接互转）。

GCJ-02 的境内判定沿用业界通用矩形范围（经度 72.004–137.8347，纬度 0.8293–55.8271），范围外不加偏。

### 3.4 局部工作平面

缓冲和叠加需要平面上的鲁棒布尔运算。做法：

- 以数据 bbox 中心为中央经线建一个横轴墨卡托平面（k0 = 1，原点移到中心），作为**拓扑工作区**；
- 顶点先在椭球上算好再投上去；两顶点之间的边在平面里是直线，而真实的边（大地线、经纬度直线、偏移曲线）在平面里略弯。按 `tolerance`（默认 1 cm）解析地加密，保证弦与真实曲线的偏差不超过容差；
- i_overlay 把浮点坐标映射到约 2³⁰ 的整数网格，1000 km 范围的量化步长约 1 mm；
- 单个几何离中心不能超过约 3900 km（覆盖全国范围），否则报错提示切分。

### 3.5 缓冲区：顶点在椭球上精确求出，平面只负责拓扑

turf 的问题是“整个几何放进一个投影里按平面距离缓冲”。默认方法 `geodesic` 反过来做：

1. **条带**：沿每条边采样，在每个采样点沿边的**局部法方向**做大地线正算，得到左右两侧距离恰为 d 的偏移点。`planar` 边的采样点和切向方位角都按经纬度直线解析计算，所以不需要先把边切碎。
2. **扇形**：每个内部折点在凸侧补一段扇形，扇形两条边与相邻条带的端边完全重合。
3. **端点圆**：开放线的两个端点补完整的大地线圆，圆的顶点与条带角点对齐。
4. 面缓冲再加上面本身；负缓冲为“面 − 边界邻域”。
5. 所有块投到工作平面，用 i_overlay 一次性求并（非零环绕规则），再反算回经纬度。

**覆盖性**：对任意折线，条带 ∪ 凸侧扇形 ∪ 端点圆恰好等于距离 d 的邻域。若某点在某边上的垂足落在边外，它到该边端点的距离严格更近，沿折点归纳必然终止于某个条带、扇形或端点圆。因此短边、锐角折线都不需要额外补圆（有“短边锯齿线无空洞”的测试）。

`projected` 方法：整体投到工作平面后直接做平面缓冲，速度快，误差约 `r²/2R²`（r 为离中心距离），适合几十公里以内。

### 3.6 叠加分析

- 两个输入投到同一个工作平面（按 `edges` 加密），i_overlay 非零规则运算（不规范的自重叠多面也能正确处理），结果反算回经纬度。
- **包围盒快速路径**：两个输入明显不相交时直接返回（相交为空、差集为 a、合并为两者拼接），不做投影。这样相距很远的几何也不会触发范围限制。
- **清理加密点**：结果中因加密产生的共线顶点会被移除（容差 = 1.5 × tolerance + 量化步长），两个经纬度矩形相交的结果仍是 5 个顶点的矩形。
- 输入先通过 `crs` 统一到 WGS84，从根源上消除坐标系混叠的偏移。

### 3.7 最近点 / 点到线距离

Karney 截距算法：以当前估计点为中心做**椭球心射投影**（大地线在其中近似为直线），在平面上求垂足，反算回椭球作为新中心，迭代至收敛（亚毫米）。长于 2000 km 的边先切分；心射投影不适用时（点在半球之外）退回采样法。`planar` 边先加密再计算，返回的 `index` 映射回原始顶点序号。

### 3.8 预处理几何与批量查询（v1.1）

同一个几何被反复查询时，最大的开销不是算法而是**每次调用都重新解析 GeoJSON**（500 顶点的面单次 273 µs，其中几何计算不到 1 µs）。

`Prepared` 把几何解析一次留在 WASM 内存里，并建立一张经纬度均匀网格索引：

- **点在面内**：只扫描查询点所在网格行的线段做射线法（边仍按经纬度直线解释）；
- **最近点**：单元格按到查询点的距离最近优先遍历，先用球面粗筛（以查询点纬度的高斯曲率半径为半径，误差 0.1% 量级），再对候选做精确的椭球求解；
- **批量接口**：`containsMany` / `nearestMany` / `distanceMany` / `within` 一次调用处理整个 `Float64Array`，把每次调用 1 µs 左右的跨边界开销摊掉。

详细的剖析数据与优化清单见 [PERFORMANCE.md](PERFORMANCE.md)。

### 3.9 拓扑验证与修复（v1.1）

分两层：

- **OGC 有效性**：自交、环不闭合、点数不足、孔不在壳内、环相交、多部件重叠——由 `geo` 的 validation 给出具体的环与坐标索引；
- **GeoJSON 约定**：RFC 7946 环绕方向、经纬度范围、非有限值、重复点、零宽尖刺、跨 180° 经线。

`makeValid` 按"可安全自动修复"的顺序处理：捕捉取整（可选）→ 去重/去共线 → 闭合环 → 非零环绕自并解消自交与重叠 → 丢弃过小部件（可选）→ 重新定向，并返回做了哪些修改的清单。

**面覆盖质量**：两两包围盒预筛后求交得到重叠；缝隙分两种——被完全围住的空洞直接取并集结果的内环；两端开口的狭缝用**形态学闭运算**（先膨胀再腐蚀，尖角用斜接保持）后与并集相减得到，并按面积阈值滤掉数值碎屑。

**线网络**：端点按容差聚类成节点（网格哈希），度为 1 的是悬挂点，度为 2 且来自两条线的是伪节点；再检测线自交、不同线之间"没有节点的交叉"、重复线（正反向归一化后哈希）、欠头（端点离另一条线在容差内却未连接）与过头（悬挂端点到本线最近交点的残段）。

**捕捉**：`snapRound` 在局部平面上把坐标取整到米制格网；`snapTo` 先吸附到参考层的顶点，再吸附到参考层的边（用同一套网格索引找最近边）。

---

## 4. API（TypeScript）

```ts
import * as gp from 'geoprecise';
await gp.init();                                 // 加载 wasm，一次即可（Node 下自动读文件）

gp.distance(a, b, { units, crs });               // 默认 km
gp.bearing(a, b, { final, crs });
gp.destination(origin, dist, bearing, { units, crs, properties });
gp.midpoint(a, b, { crs });
gp.length(geojson, { units, crs, edges });
gp.area(geojson, { crs, edges });                // m²
gp.along(line, dist, { units, crs, edges });
gp.nearestPointOnLine(line, pt, { units, crs, edges });  // dist / location / index / multiFeatureIndex
gp.pointToLineDistance(pt, line, { units, crs, edges });
gp.circle(center, radius, { units, steps, crs, properties });

gp.buffer(geojson, radius, { units, steps, method, edges, tolerance, crs });
gp.intersect(a, b, opts) | gp.intersect(featureCollection, opts)   // 两种调用方式都支持
gp.union / gp.difference / gp.xor / gp.unionAll
gp.booleanPointInPolygon(pt, polygon, { ignoreBoundary });
gp.booleanIntersects(a, b);

// v1.1：预处理几何与批量接口
const prep = gp.prepare(polygon, { units: 'meters' });
prep.contains(pt); prep.containsMany(float64Array); prep.nearest(pt);
prep.nearestMany(pts); prep.distanceMany(pts); prep.within(pts, 500); prep.free();
gp.distanceBatch(pairs); gp.distanceToBatch(origin, pts); gp.destinationBatch(rows);

// v1.1：几何处理
gp.simplify(geojson, 5, { units: 'meters', preserveTopology });
gp.convexHull(fc); gp.concaveHull(fc, { maxEdge: 2 });
gp.centroid(g); gp.centerOfMass(g); gp.pointOnFeature(g);
gp.bbox(g); gp.bboxPolygon(b); gp.bboxClip(g, b);
gp.transformRotate(g, 45, { pivot }); gp.transformTranslate(g, 2, 90); gp.transformScale(g, 1.5);
gp.lineSlice(start, stop, line); gp.lineSliceAlong(line, 0, 10); gp.lineChunk(line, 5);
gp.lineIntersect(a, b); gp.greatCircle(a, b); gp.sector(c, 10, 0, 90);
gp.nearestPoint(target, points); gp.pointsWithinPolygon(points, polygon);
gp.flatten(g); gp.explode(g); gp.polygonToLine(g); gp.lineToPolygon(g);
gp.rewind(g); gp.cleanCoords(g); gp.truncate(g, { precision: 6 }); gp.dissolve(fc, { propertyName });

// v1.1：DE-9IM 谓词
gp.relate(a, b);                                 // "212101212"
gp.relatePattern(a, b, 'T*****FF*');
gp.booleanContains / booleanWithin / booleanCrosses / booleanTouches /
gp.booleanOverlap / booleanDisjoint / booleanEqual / booleanRelation(a, b, 'coveredBy');
gp.booleanPointOnLine(pt, line, { tolerance: 0.5 });

// v1.1：拓扑验证与质量检查
gp.validate(geojson);                            // { valid, issues[] }
gp.makeValid(geojson, { snapGrid, minArea });    // 结果带 geoprecise:fixes
gp.coverageIssues(fc, { gapTolerance: 0.5 });    // 重叠 + 缝隙
gp.networkIssues(fc, { tolerance: 0.05, maxUndershoot: 2, maxOvershoot: 1 });
gp.snapRound(g, 0.01); gp.snapTo(g, reference, 0.5);

gp.transform(geojson, from, to);                 // 保留 id、属性、Z 值
gp.convert(position, from, to);
gp.transformCoords(float64Array, from, to, stride);        // 原地转换
gp.gcj02ToWgs84 / wgs84ToGcj02 / bd09ToWgs84 / wgs84ToBd09 / gcj02ToBd09 / bd09ToGcj02
gp.gaussKrugerCrs(lon, { zoneWidth, zonePrefix });         // → 'EPSG:45xx'
gp.utmCrs(lon, lat); gp.normalizeCrs(crs); gp.version();
```

---

## 5. 工程结构与构建

```
geoprecise/
├─ Cargo.toml                 workspace（Rust ≥ 1.88）
├─ crates/core                纯 Rust 核心 + 单元测试 + GeographicLib/PROJ 对照测试
├─ crates/wasm                wasm-bindgen 导出
├─ packages/geoprecise        TS 封装（npm 包，含 wasm/ 与 dist/）
├─ examples/web-demo          MapLibre 对比页（Vite），可切换 OSM / 高德底图
├─ bench                      基准数据生成、精度与速度对比、pyproj 独立复核
├─ scripts/build-wasm.sh      cargo → wasm-bindgen → wasm-opt
└─ docs/                      本文、精度报告
```

产物体积：wasm 2.04 MB（gzip 后 668 KB，未经 `wasm-opt`）。构成与瘦身方案见 [PERFORMANCE.md](PERFORMANCE.md) 第 4、5 节。

---

## 6. turf 覆盖层（v1.2）

v1.2 的目标是"能整包替换 turf"，所以补齐的标准是 **turf 的每一个导出都有同名实现**：184 / 184。
逐函数对照与实测差异见 [TURF-COVERAGE.md](TURF-COVERAGE.md)。

新增的 Rust 模块：

| 模块 | 内容 | 关键取舍 |
|---|---|---|
| `rhumb` | 恒向线距离 / 方位 / 目标点 | 子午线弧长用 7 点 Gauss–Legendre 积分（对涉及的跨度已到舍入精度），反算用 Newton 迭代，不用截断级数 |
| `lines` | 分段、按几何切分、单侧偏移、重叠段、点到面距离、夹角 | 偏移在凸角生成圆弧（顶点严格等距），凹角保留两个垂足而不做尖角延伸——避免尖刺，代价是凹角顶点落在 `d·cos(转角)` |
| `grids` | 点 / 方 / 矩形 / 三角 / 六边形格网 | 在局部横轴墨卡托平面布点，格网尺寸是真实米；turf 把尺寸换算成度后两轴共用，纬度一高就东西向拉伸 |
| `shapes` | 椭圆、平滑、切线、掩膜、凹凸判断、平行判断、翻转、均值/中位中心、样条、成面 | `polygonize` 用打断 + 平面半边图 + 最小面遍历 + 面积符号筛选；中位中心用 Weiszfeld 在米制平面上迭代 |
| `interp` | IDW 插值、等值线、等值面、TIN、Voronoi、三角面内插 | 等值线**按格网三角形**求解：三角形上的线性插值只有唯一交点，marching squares 需要靠单元均值猜的鞍点歧义根本不出现。等值面用三角形裁剪 + 并集，band 之间无缝无叠 |
| `cluster` | DBSCAN、k-means、最近邻分析、标准差椭圆、方向均值、绕障最短路 | 全部在米制平面拟合。k-means 用固定发生器做 k-means++ 播种，结果可复现；最短路 A* 之后做 string pulling，无障碍时直接退化为直线 |
| `stats` | 空间权重、Moran's I、样方分析、三角化 | 邻域半径是大地线距离，样方等面积；turf 这两处都在度空间里做，结论会随纬度漂移 |

三个值得单独记下的实现细节：

**TIN 的退化三角形。** 在局部平面上三角化的好处是三角形形状在地面上合理，但代价是：
经纬度格网的边界行在经纬度里严格共线，投到平面后是极轻微的凸弧，Delaunay 会在那里
吐出一扇零面积三角形（平面里它们有几十 m²，看不出异常）。判定退化必须在**输出空间**做——
按经纬度 shoelace 面积、阈值取输入范围的 1e-9。加上这一步后三角形数量与 turf 完全一致
（400 vs 400），且仍然严格铺满凸包。

**Voronoi 的发丝缝。** 单元格由"对每个 Delaunay 邻居做垂直平分半平面裁剪"得到，
在平面里是精确的；但写成经纬度顶点后，平分线变成弦，相邻单元之间留下约 1e-4 面积占比的
发丝缝（30 km 尺度）。这是把平面直线写进 GeoJSON 的固有代价，选择记录下来而不是靠加密
顶点掩盖。

**vertexCentroid 的闭合点。** turf 的 `centroid` 排除环的闭合点，v1.1 之前我们把它算进了
均值，会把结果拉向第一个角点（正方形上偏 1/4 边长）。v1.2 修正，并补了对照测试。

---

## 7. 已知限制（v1.2）

- 跨 180° 经线的几何不做特殊处理。
- 单个几何需在中央经线两侧约 3900 km 内（全国范围可用），更大范围请先切分。
- GCJ-02 境内判定为矩形范围，边境附近可能与官方实现不同。
- BD-09 为公开算法，与百度官方接口可能存在亚米级差异；百度墨卡托（BD09MC）未支持。
- `planar` 语义下，缓冲半径超过约 1000 km 且位于高纬度时，偏移曲线假设不再严格成立。
- 默认模式下，大范围面缓冲比 turf 慢 3~7 倍（几十毫秒）。
- 点在面内等谓词直接在输入坐标里做平面判断（两者须同一坐标系）。
- 缝隙检测的闭运算会把"本来就很窄的凹口"也算作缝隙，需要按 `minGap` / `maxGap` 调阈值。
- `snapTo` 只移动被捕捉图层的顶点，不会在参考层上插入节点（完整的共边一致化留待 v2）。
- `makeValid` 处理面的自交与重叠；线的自交只做去重，不会在交点处打断。
- Voronoi 单元的共享边是平面直线写成的经纬度弦，相邻单元之间有约 1e-4 面积占比的发丝缝（见上）。
- `lineOffset` 在凹角不做尖角延伸，凹角顶点落在 `d·cos(转角)` 而不是 `d`。
- `quadratAnalysis` 的 χ² 只检验聚集一侧：完全规则的格网同样"不被拒绝"，需要看 `varianceMeanRatio` 才能区分离散与随机。
- `radiansToLength` / `lengthToRadians` / `lengthToDegrees` 按定义是球面量，保留是为了与 turf 兼容；测量请用 `distance`。
- `geojsonRbush` 是均匀网格而非真正的 R 树，接口与结果一致，极端不均匀分布下查询效率不如 R 树。
- binaryen < 116 的 `wasm-opt` 会把 externref 表重编号而不修正导出，构建脚本检测到就跳过优化。

## 8. 后续路线

| 版本 | 内容 |
|---|---|
| v1.1 | 预处理几何 + 网格索引、批量接口；几何处理与 DE-9IM 谓词；拓扑验证、面覆盖与线网络检查、捕捉；Clenshaw 投影、闭式平面面积/长度 |
| v1.2（本次） | turf 184 个导出全覆盖：恒向线、线工具、米制格网、形状构造、插值与等值线、TIN / Voronoi、聚类与点模式统计、绕障最短路、纯 TS 助手层与 `geojsonRbush` |
| v1.3 | `Prepared` 上补 area/length/buffer 复用；二进制几何通道；`precision: 'fast'` 快速测量模式；Web Worker 封装；cargo feature 瘦身 |
| v2 | 跨 180° 支持；线自交打断与共边一致化；Voronoi 共享边加密消除发丝缝；GCJ-02 精细国界判定；BD09MC |
| v2.x | 与 fleet-tracking 等 Web 地图项目对接：电子围栏、轨迹缓冲、道路匹配前处理 |
