// JS ↔ WASM boundary cost breakdown.
import { readFileSync } from 'node:fs';
import * as gp from '../packages/geoverse-precise/dist/index.js';
import * as wasm from '../packages/geoverse-precise/wasm/geoverse_precise_wasm.js';

await gp.init();
const bench = (name, reps, fn) => {
  fn();
  const t0 = performance.now();
  let r;
  for (let i = 0; i < reps; i++) r = fn();
  const per = (performance.now() - t0) / reps;
  console.log(`${name.padEnd(52)} ${(per * 1000).toFixed(3).padStart(10)} µs`);
  return r;
};

console.log('--- per-call overhead ---');
bench('wasm.version() (no args)', 200000, () => wasm.version());
bench('wasm.distance(4 numbers + 2 strings)', 200000, () => wasm.distance(116, 39, 121, 31, 'kilometers', ''));
bench('wasm.distance(4 numbers + 2 empty strings)', 200000, () => wasm.distance(116, 39, 121, 31, '', ''));
bench('gp.distance (TS wrapper, array input)', 200000, () => gp.distance([116, 39], [121, 31]));
bench('gp.distance (units: meters)', 200000, () => gp.distance([116, 39], [121, 31], { units: 'meters' }));
bench('gp.distance (crs: GCJ02)', 200000, () => gp.distance([116, 39], [121, 31], { crs: 'GCJ02' }));

console.log('\n--- geometry marshalling ---');
const mkPoly = (n) => {
  const ring = [];
  for (let i = 0; i < n; i++) {
    const a = (2 * Math.PI * i) / n;
    ring.push([120 + 0.5 * Math.cos(a), 30 + 0.5 * Math.sin(a)]);
  }
  ring.push(ring[0]);
  return { type: 'Polygon', coordinates: [ring] };
};
for (const n of [100, 1000, 10000]) {
  const poly = mkPoly(n);
  const str = JSON.stringify(poly);
  console.log(`  polygon with ${n} vertices (${(str.length / 1024).toFixed(0)} KB JSON)`);
  bench('    JSON.stringify only', 200, () => JSON.stringify(poly));
  bench('    JSON.parse only', 200, () => JSON.parse(str));
  bench('    wasm.area(str)  [parse + compute]', 200, () => wasm.area(str, '', ''));
  bench('    gp.area(object) [stringify + parse + compute]', 200, () => gp.area(poly));
  bench('    gp.transform (round trip, out too)', 50, () => gp.transform(poly, 'WGS84', 'EPSG:4549'));
}

console.log('\n--- bulk point workloads (current API) ---');
const N = 20000;
const pts = Array.from({ length: N }, (_, i) => [117 + (i % 200) * 0.005, 31 + Math.floor(i / 200) * 0.005]);
const poly = mkPoly(500);
bench(`${N} × gp.distance to a fixed point`, 3, () => {
  let s = 0;
  for (const p of pts) s += gp.distance(p, [120, 30], { units: 'meters' });
  return s;
});
bench(`${N} × gp.booleanPointInPolygon (500-vertex)`, 1, () => {
  let c = 0;
  for (const p of pts) if (gp.booleanPointInPolygon(p, poly)) c++;
  return c;
});
const line = { type: 'LineString', coordinates: Array.from({ length: 280 }, (_, i) => [116 + i * 0.02, 39 - i * 0.03]) };
bench(`200 × gp.pointToLineDistance (280-segment line)`, 1, () => {
  let s = 0;
  for (let i = 0; i < 200; i++) s += gp.pointToLineDistance(pts[i], line, { units: 'meters' });
  return s;
});
const flat = new Float64Array(N * 2);
pts.forEach((p, i) => { flat[2 * i] = p[0]; flat[2 * i + 1] = p[1]; });
bench(`${N} × transformCoords (batch, in place)`, 20, () => gp.transformCoords(Float64Array.from(flat), 'WGS84', 'EPSG:4549'));

console.log('\n--- prepared geometry & batch APIs ---');
{
  const poly = mkPoly(500);
  const prep = gp.prepare(poly);
  bench('gp.prepare(500-vertex polygon)', 50, () => {
    const p = gp.prepare(poly);
    p.free();
  });
  bench(`${N} × contains, prepared (per call)`, 2, () => {
    let c = 0;
    for (const p of pts) if (prep.contains(p)) c++;
    return c;
  });
  const flatPts = new Float64Array(N * 2);
  pts.forEach((p, i) => { flatPts[2 * i] = p[0]; flatPts[2 * i + 1] = p[1]; });
  bench(`${N} × contains, prepared (one batch call)`, 20, () => prep.containsMany(flatPts));
  const linePrep = gp.prepare(line);
  bench(`2000 × nearest on line, prepared (batch)`, 5, () => linePrep.nearestMany(flatPts.subarray(0, 4000)));
  bench(`${N} × distance to point (batch call)`, 20, () => gp.distanceToBatch([120, 30], flatPts, { units: 'meters' }));
  prep.free();
  linePrep.free();
}
