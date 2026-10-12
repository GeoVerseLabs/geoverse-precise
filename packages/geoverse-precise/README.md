# geoverse-precise

[![npm](https://img.shields.io/npm/v/@geoverse/geoverse-precise)](https://www.npmjs.com/package/@geoverse/geoverse-precise)
[![CI](https://github.com/GeoVerseLabs/geoverse-precise/actions/workflows/ci.yml/badge.svg)](https://github.com/GeoVerseLabs/geoverse-precise/actions/workflows/ci.yml)

Ellipsoid-accurate spatial analysis for the browser and Node.js, written in Rust and compiled to WebAssembly.
A drop-in replacement for [turf.js](https://turfjs.org/) — all 184 turf exports exist under the same names —
that measures on the WGS84 ellipsoid instead of a sphere and understands China's coordinate systems
(CGCS2000, GCJ-02, BD-09, Gauss–Krüger zones).

椭球精度的浏览器端空间分析库（Rust → WebAssembly），API 与 turf.js 同名对齐，支持 CGCS2000 / GCJ-02 / BD-09 等国内坐标系。
完整中文文档见 [GitHub 仓库](https://github.com/GeoVerseLabs/geoverse-precise#readme)。

## Install

```bash
npm install @geoverse/geoverse-precise
```

Each version is also attached to its [GitHub Release](https://github.com/GeoVerseLabs/geoverse-precise/releases) with SHA-256 checksums.

ESM only. Node.js ≥ 20, or any bundler that handles `new URL('…wasm', import.meta.url)` (Vite, webpack 5, Rollup, esbuild).

## Usage

```ts
import * as gp from '@geoverse/geoverse-precise';

await gp.init(); // loads the .wasm once (fetch in browsers, file read in Node)

gp.distance([116.397, 39.909], [121.474, 31.23]);   // 1066.78 km on the WGS84 ellipsoid
gp.area(polygon);                                     // m²
gp.buffer(line, 500, { units: 'meters' });            // a true 500 m buffer at any latitude
gp.intersect(a, b);                                   // Feature | null
gp.transform(featureCollection, 'GCJ02', 'WGS84');    // Amap / Tencent coordinates → WGS84
gp.gaussKrugerCrs(120.3);                             // 'EPSG:4549' (CGCS2000 3° zone)
```

Measuring and constructing functions accept `crs` (the input coordinate system, e.g. `{ crs: 'GCJ02' }`);
length, area, buffer and overlay also take `edges: 'planar' | 'geodesic'`.
Defaults match turf: kilometres, and edges drawn as straight lon/lat lines.

For many queries against one geometry, parse it once:

```ts
const zone = gp.prepare(polygon, { units: 'meters' });
const inside = zone.containsMany(new Float64Array([116.4, 39.9, 121.5, 31.2])); // Uint8Array
zone.free();
```

## Migrating from turf

Replace `import * as turf from '@turf/turf'` with `import * as turf from '@geoverse/geoverse-precise'` and call
`await turf.init()` once at start-up. Results differ from turf where turf is spherical or works in degrees
(distances, areas, buffers, grids); see the
[function-by-function comparison](https://github.com/GeoVerseLabs/geoverse-precise/blob/main/docs/TURF-COVERAGE.md)
and the [accuracy report](https://github.com/GeoVerseLabs/geoverse-precise/blob/main/docs/ACCURACY.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
