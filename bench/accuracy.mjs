// Accuracy & speed comparison: turf.js / gcoord / proj4js vs geoverse-precise,
// against GeographicLib and PROJ reference data (bench/fixtures, see gen_fixtures.py).
//
//   cd packages/geoverse-precise && npm run build && cd ../../bench && npm i && node accuracy.mjs
//
// Writes bench/out/report.json and bench/out/buffers.json (vertices for the
// independent Python cross-check in verify_buffer.py).
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import * as turf from '@turf/turf';
import gcoord from 'gcoord';
import coordtransform from 'coordtransform';
import proj4 from 'proj4';
import * as gp from '../packages/geoverse-precise/dist/index.js';

await gp.init();
const here = new URL('.', import.meta.url);
const load = (n) => JSON.parse(readFileSync(new URL(`fixtures/${n}`, here)));
mkdirSync(new URL('out/', here), { recursive: true });

const stats = (arr) => {
  const a = arr.map(Math.abs).sort((x, y) => x - y);
  const q = (p) => a[Math.min(a.length - 1, Math.floor(p * a.length))];
  return { n: a.length, median: q(0.5), p95: q(0.95), max: a[a.length - 1] };
};
const time = (fn, reps = 1) => {
  fn();
  const t0 = performance.now();
  let r;
  for (let i = 0; i < reps; i++) r = fn();
  return { ms: (performance.now() - t0) / reps, result: r };
};
const report = {};

// ---------------------------------------------------------------- distance
{
  const rows = load('inverse.json');
  const byTag = {};
  for (const r of rows) {
    const a = [r.lon1, r.lat1], b = [r.lon2, r.lat2];
    const t = turf.distance(a, b, { units: 'meters' }) - r.s12;
    const g = gp.distance(a, b, { units: 'meters' }) - r.s12;
    const k = (byTag[r.tag] ??= { turf: [], turfRel: [], gp: [] });
    k.turf.push(t);
    k.turfRel.push(r.s12 > 0 ? t / r.s12 : 0);
    k.gp.push(g);
  }
  report.distance = Object.fromEntries(
    Object.entries(byTag).map(([tag, v]) => [
      tag,
      { turf_m: stats(v.turf), turf_rel: stats(v.turfRel), geoverse_precise_m: stats(v.gp) },
    ]),
  );
}

// ------------------------------------------------------------- destination
{
  const rows = load('direct.json');
  const t = [], g = [];
  for (const r of rows) {
    const tp = turf.destination([r.lon, r.lat], r.s, r.azi, { units: 'meters' }).geometry.coordinates;
    const gpp = gp.destination([r.lon, r.lat], r.s, r.azi, { units: 'meters' }).geometry.coordinates;
    t.push(gp.distance(tp, [r.lon2, r.lat2], { units: 'meters' }));
    g.push(gp.distance(gpp, [r.lon2, r.lat2], { units: 'meters' }));
  }
  report.destination = { turf_m: stats(t), geoverse_precise_m: stats(g) };
}

// -------------------------------------------------------------------- area
{
  // Reference polygons have geodesic edges; for small polygons edge semantics are
  // irrelevant, for large ones turf's formula implies different edges.
  const rows = load('area.json');
  const t = { small: [], large: [] }, g = { small: [], large: [] };
  for (const r of rows) {
    const k = r.radius < 20000 ? 'small' : 'large';
    const poly = { type: 'Polygon', coordinates: [r.ring] };
    t[k].push((turf.area(poly) - r.area) / r.area);
    g[k].push((gp.area(poly, { edges: 'geodesic' }) - r.area) / r.area);
  }
  report.area = {
    'radius<20km': { turf_rel: stats(t.small), geoverse_precise_rel: stats(g.small) },
    'radius 20–800km': { turf_rel: stats(t.large), geoverse_precise_rel: stats(g.large) },
  };
}

// ------------------------------------------------------------- projections
{
  const rows = load('projections.json');
  const defs = {};
  const def = (code) => {
    if (defs[code]) return defs[code];
    const c = Number(code.split(':')[1]);
    let d;
    if (c === 3857) d = 'EPSG:3857';
    else if (c >= 32601 && c <= 32660) d = `+proj=utm +zone=${c - 32600} +datum=WGS84 +units=m +no_defs`;
    else if (c >= 32701 && c <= 32760) d = `+proj=utm +zone=${c - 32700} +south +datum=WGS84 +units=m +no_defs`;
    else {
      let lon0, x0;
      if (c <= 4501) { const z = c - 4491 + 13; lon0 = 6 * z - 3; x0 = z * 1e6 + 5e5; }
      else if (c <= 4512) { const z = c - 4502 + 13; lon0 = 6 * z - 3; x0 = 5e5; }
      else if (c <= 4533) { const z = c - 4513 + 25; lon0 = 3 * z; x0 = z * 1e6 + 5e5; }
      else { const z = c - 4534 + 25; lon0 = 3 * z; x0 = 5e5; }
      d = `+proj=tmerc +lat_0=0 +lon_0=${lon0} +k=1 +x_0=${x0} +y_0=0 +ellps=GRS80 +units=m +no_defs`;
    }
    return (defs[code] = d);
  };
  const p4 = { zone: [], wide: [] }, g = { zone: [], wide: [] };
  for (const r of rows) {
    const k = r.wide ? 'wide' : 'zone';
    const [x1, y1] = proj4('EPSG:4326', def(r.dst), [r.lon, r.lat]);
    p4[k].push(Math.hypot(x1 - r.x, y1 - r.y));
    const [x2, y2] = gp.convert([r.lon, r.lat], r.src, r.dst);
    g[k].push(Math.hypot(x2 - r.x, y2 - r.y));
  }
  report.projection = {
    inZone: { proj4js_m: stats(p4.zone), geoverse_precise_m: stats(g.zone) },
    wide6deg: { proj4js_m: stats(p4.wide), geoverse_precise_m: stats(g.wide) },
  };
  // batch speed
  const N = 200000;
  const flat = new Float64Array(N * 2);
  for (let i = 0; i < N; i++) { flat[2 * i] = 118 + (i % 1000) * 0.004; flat[2 * i + 1] = 28 + (i % 997) * 0.004; }
  const tp = time(() => { for (let i = 0; i < N; i++) proj4('EPSG:4326', def('EPSG:4549'), [flat[2 * i], flat[2 * i + 1]]); });
  const tg = time(() => gp.transformCoords(Float64Array.from(flat), 'WGS84', 'EPSG:4549'));
  report.projection.speed_200k_points_ms = { proj4js: tp.ms, geoverse_precise_batch: tg.ms };
}

// --------------------------------------------------------------- GCJ-02
{
  const fwdDiff = [], invG = [], invGp = [], invCt = [];
  let seed = 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  for (let i = 0; i < 2000; i++) {
    const w = [75 + rnd() * 59, 19 + rnd() * 34];
    const g1 = gcoord.transform(w, gcoord.WGS84, gcoord.GCJ02);
    const g2 = gp.wgs84ToGcj02(w);
    fwdDiff.push(gp.distance(g1, g2, { units: 'meters' }));
    const back1 = gcoord.transform(g2, gcoord.GCJ02, gcoord.WGS84);
    const back2 = gp.gcj02ToWgs84(g2);
    invG.push(gp.distance(back1, w, { units: 'meters' }));
    invCt.push(gp.distance(coordtransform.gcj02towgs84(g2[0], g2[1]), w, { units: 'meters' }));
    invGp.push(gp.distance(back2, w, { units: 'meters' }));
  }
  const bdG = [], bdGp = [];
  for (let i = 0; i < 2000; i++) {
    const w = [75 + rnd() * 59, 19 + rnd() * 34];
    const b = gp.wgs84ToBd09(w);
    bdG.push(gp.distance(gcoord.transform(b, gcoord.BD09, gcoord.WGS84), w, { units: 'meters' }));
    bdGp.push(gp.distance(gp.bd09ToWgs84(b), w, { units: 'meters' }));
  }
  report.china = {
    gcj02_forward_gcoord_vs_geoverse_precise_m: stats(fwdDiff),
    gcj02_to_wgs84_roundtrip_m: { coordtransform: stats(invCt), gcoord: stats(invG), geoverse_precise: stats(invGp) },
    bd09_to_wgs84_roundtrip_m: { gcoord: stats(bdG), geoverse_precise: stats(bdGp) },
  };
}

// ------------------------------------------------------------------ buffer
const dump = {};
{
  const vertsOf = (f) => {
    const g = f.geometry;
    const polys = g.type === 'Polygon' ? [g.coordinates] : g.coordinates;
    return polys.flatMap((p) => p[0].slice(0, -1));
  };
  // Distance stick: geoverse-precise geodesic nearest-point (validated against brute force
  // in Rust tests and against pyproj in verify_buffer.py).
  const errToLine = (verts, line, d) =>
    verts.map((v) => gp.pointToLineDistance(v, line, { units: 'meters', edges: 'geodesic' }) - d);
  const errToPoint = (verts, c, d) => verts.map((v) => gp.distance(c, v, { units: 'meters' }) - d);

  // densified geodesic route so that edge semantics don't matter (< 1 cm)
  const route = (a, b, stepKm) => {
    const n = Math.ceil(gp.distance(a, b) / stepKm);
    const br = gp.bearing(a, b);
    const total = gp.distance(a, b);
    const pts = [a];
    for (let i = 1; i < n; i++) pts.push(gp.destination(a, (total * i) / n, br).geometry.coordinates);
    pts.push(b);
    return pts;
  };

  const scenarios = [];
  scenarios.push({ name: 'point 1 km @Beijing', geom: { type: 'Point', coordinates: [116.397, 39.909] }, d: 1000 });
  scenarios.push({ name: 'point 50 km @Harbin', geom: { type: 'Point', coordinates: [126.63, 45.75] }, d: 50000 });
  const bjsh = [...route([116.397, 39.909], [117.2, 36.65], 4), ...route([117.2, 36.65], [118.8, 32.06], 4).slice(1), ...route([118.8, 32.06], [121.47, 31.23], 4).slice(1)];
  scenarios.push({ name: 'route 2 km, Beijing→Shanghai (~1100 km)', geom: { type: 'LineString', coordinates: bjsh }, d: 2000 });
  const ring = [
    ...route([112, 30], [118, 30.5], 4),
    ...route([118, 30.5], [117.5, 35], 4).slice(1),
    ...route([117.5, 35], [111.5, 34.5], 4).slice(1),
    ...route([111.5, 34.5], [112, 30], 4).slice(1),
  ];
  scenarios.push({ name: 'polygon 5 km, ~550 km wide', geom: { type: 'Polygon', coordinates: [ring] }, d: 5000 });

  report.buffer = {};
  for (const s of scenarios) {
    // edges: 'geodesic' matches the geodesic distance stick (inputs have ≤4 km edges,
    // where turf's AEQD-straight edges are within centimetres of geodesics).
    const base = { units: 'meters', steps: 16, edges: 'geodesic' };
    const tt = time(() => turf.buffer(s.geom, s.d, { units: 'meters', steps: 16 }), 3);
    const tg = time(() => gp.buffer(s.geom, s.d, base), 3);
    const tg5 = time(() => gp.buffer(s.geom, s.d, { ...base, tolerance: 0.05 }), 3);
    const tpl = time(() => gp.buffer(s.geom, s.d, { units: 'meters', steps: 16 }), 3);
    const tp = time(() => gp.buffer(s.geom, s.d, { ...base, method: 'projected' }), 3);
    const [vt, vg, vp] = [vertsOf(tt.result), vertsOf(tg.result), vertsOf(tp.result)];
    let et, eg, ep, line;
    if (s.geom.type === 'Point') {
      et = errToPoint(vt, s.geom.coordinates, s.d);
      eg = errToPoint(vg, s.geom.coordinates, s.d);
      ep = errToPoint(vp, s.geom.coordinates, s.d);
    } else {
      line = s.geom.type === 'Polygon' ? { type: 'LineString', coordinates: s.geom.coordinates[0] } : s.geom;
      et = errToLine(vt, line, s.d);
      eg = errToLine(vg, line, s.d);
      ep = errToLine(vp, line, s.d);
    }
    const v5 = vertsOf(tg5.result);
    const e5 = s.geom.type === 'Point' ? errToPoint(v5, s.geom.coordinates, s.d) : errToLine(v5, line, s.d);
    report.buffer[s.name] = {
      vertices: { turf: vt.length, geodesic: vg.length, geodesic_tol5cm: v5.length, projected: vp.length },
      error_m: { turf: stats(et), geodesic: stats(eg), geodesic_tol5cm: stats(e5), projected: stats(ep) },
      error_rel_max: {
        turf: stats(et).max / s.d,
        geodesic: stats(eg).max / s.d,
        projected: stats(ep).max / s.d,
      },
      time_ms: { turf: tt.ms, geodesic: tg.ms, geodesic_tol5cm: tg5.ms, geodesic_planar_edges: tpl.ms, projected: tp.ms },
    };
    dump[s.name] = { d: s.d, input: s.geom, turf: vt, geodesic: vg, projected: vp, stick: { turf: et, geodesic: eg, projected: ep } };
  }
}

// ------------------------------------------------------------- measurement speed
{
  const pts = Array.from({ length: 100000 }, (_, i) => [[110 + (i % 100) * 0.01, 30 + (i % 77) * 0.01], [111 + (i % 50) * 0.02, 31]]);
  const t = time(() => { let s = 0; for (const [a, b] of pts) s += turf.distance(a, b); return s; });
  const g = time(() => { let s = 0; for (const [a, b] of pts) s += gp.distance(a, b); return s; });
  report.speed_100k_distance_ms = { turf_haversine: t.ms, geoverse_precise_karney: g.ms };
}

writeFileSync(new URL('out/report.json', here), JSON.stringify(report, null, 2));
writeFileSync(new URL('out/buffers.json', here), JSON.stringify(dump));
console.log(JSON.stringify(report, null, 2));
