import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import * as gp from '../dist/index.js';

before(async () => {
  await gp.init();
});

const BJ = [116.397128, 39.916527];
const SH = [121.473701, 31.230416];

test('version', () => {
  assert.match(gp.version(), /^\d+\.\d+\.\d+/);
});

test('distance / bearing / destination are consistent', () => {
  const d = gp.distance(BJ, SH); // km
  assert.ok(d > 1060 && d < 1075, `d=${d}`);
  const b = gp.bearing(BJ, SH);
  const dest = gp.destination(BJ, d, b);
  const back = gp.distance(dest, SH, { units: 'meters' });
  assert.ok(back < 1e-6, `back=${back}`);
  assert.equal(dest.type, 'Feature');
});

test('accepts Point geometries and features', () => {
  const a = gp.distance({ type: 'Point', coordinates: BJ }, { type: 'Feature', properties: {}, geometry: { type: 'Point', coordinates: SH } });
  assert.equal(a, gp.distance(BJ, SH));
});

test('area of 1°×1° cell at the equator', () => {
  const poly = { type: 'Polygon', coordinates: [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]] };
  // GeographicLib reference for geodesic edges
  assert.ok(Math.abs(gp.area(poly, { edges: 'geodesic' }) - 12308778361.469452) < 1e-3);
  // Planar edges: the northern edge follows the parallel, so the cell is slightly smaller.
  const planar = gp.area(poly);
  assert.ok(planar < 12308778361.469452 && planar > 12308778361.469452 * 0.9999, `${planar}`);
});

test('circle radius is exact', () => {
  const c = gp.circle(BJ, 500, { units: 'meters', steps: 32, properties: { id: 1 } });
  assert.equal(c.geometry.coordinates[0].length, 33);
  assert.deepEqual(c.properties, { id: 1 });
  for (const p of c.geometry.coordinates[0]) {
    assert.ok(Math.abs(gp.distance(BJ, p, { units: 'meters' }) - 500) < 1e-6);
  }
});

test('buffer keeps properties and handles collections', () => {
  const fc = {
    type: 'FeatureCollection',
    features: [
      { type: 'Feature', properties: { name: 'line' }, geometry: { type: 'LineString', coordinates: [[120, 30], [120.1, 30.05], [120.2, 30]] } },
      { type: 'Feature', properties: { name: 'pt' }, geometry: { type: 'Point', coordinates: [120, 30] } },
      { type: 'Feature', properties: { name: 'tiny' }, geometry: { type: 'Polygon', coordinates: [[[121, 30], [121.001, 30], [121.001, 30.001], [121, 30]]] } },
    ],
  };
  const out = gp.buffer(fc, 200, { units: 'meters' });
  assert.equal(out.type, 'FeatureCollection');
  assert.deepEqual(out.features.map((f) => f.properties.name), ['line', 'pt', 'tiny']);
  const shrunk = gp.buffer(fc.features[2], -500, { units: 'meters' });
  assert.equal(shrunk, undefined);
});

test('buffer in GCJ02 space', () => {
  const pt = { type: 'Point', coordinates: [116.404, 39.915] };
  const b = gp.buffer(pt, 1, { crs: 'GCJ02' });
  const v = b.geometry.coordinates[0][10];
  const d = gp.distance(pt, v, { crs: 'GCJ02', units: 'meters' });
  assert.ok(Math.abs(d - 1000) < 1e-6, `d=${d}`);
});

test('overlay: two-arg and turf v7 collection forms', () => {
  const a = { type: 'Feature', properties: {}, geometry: { type: 'Polygon', coordinates: [[[120, 30], [121, 30], [121, 31], [120, 31], [120, 30]]] } };
  const b = { type: 'Feature', properties: {}, geometry: { type: 'Polygon', coordinates: [[[120.5, 30.5], [121.5, 30.5], [121.5, 31.5], [120.5, 31.5], [120.5, 30.5]]] } };
  const i1 = gp.intersect(a, b);
  const i2 = gp.intersect({ type: 'FeatureCollection', features: [a, b] });
  assert.ok(Math.abs(gp.area(i1) - gp.area(i2)) < 1e-6);
  const u = gp.union({ type: 'FeatureCollection', features: [a, b] }, { properties: { k: 'v' } });
  assert.deepEqual(u.properties, { k: 'v' });
  const expect = gp.area(a) + gp.area(b) - gp.area(i1);
  assert.ok(Math.abs(gp.area(u) - expect) / expect < 1e-6);
  const d = gp.difference(a, b);
  assert.ok(Math.abs(gp.area(d) - (gp.area(a) - gp.area(i1))) / gp.area(d) < 1e-6);
  const far = { type: 'Polygon', coordinates: [[[0, 0], [1, 0], [1, 1], [0, 0]]] };
  assert.equal(gp.intersect(a, far), null);
  assert.equal(gp.union(a, far).geometry.type, 'MultiPolygon');
  // planar semantics: intersection of two lon/lat rectangles is a lon/lat rectangle
  assert.equal(i1.geometry.coordinates[0].length, 5);
  for (const [x, y] of i1.geometry.coordinates[0]) {
    assert.ok([120.5, 121].some((v) => Math.abs(x - v) < 1e-8), `${x}`);
    assert.ok([30.5, 31].some((v) => Math.abs(y - v) < 1e-8), `${y}`);
  }
});

test('nearest point on line and point-to-line distance', () => {
  const line = { type: 'LineString', coordinates: [[116, 40], [117, 40], [117, 41]] };
  const np = gp.nearestPointOnLine(line, [116.5, 40.2], { units: 'meters' });
  assert.equal(np.properties.index, 0);
  const d = gp.pointToLineDistance([116.5, 40.2], line, { units: 'meters' });
  assert.ok(Math.abs(d - np.properties.dist) < 1e-9);
  assert.ok(Math.abs(gp.distance(np, [116.5, 40.2], { units: 'meters' }) - d) < 1e-6);
});

test('CRS conversions', () => {
  const g = gp.wgs84ToGcj02(BJ);
  const w = gp.gcj02ToWgs84(g);
  assert.ok(Math.abs(w[0] - BJ[0]) < 1e-9 && Math.abs(w[1] - BJ[1]) < 1e-9);
  const bd = gp.wgs84ToBd09(BJ);
  const w2 = gp.bd09ToWgs84(bd);
  assert.ok(Math.abs(w2[0] - BJ[0]) < 1e-9 && Math.abs(w2[1] - BJ[1]) < 1e-9);

  assert.equal(gp.gaussKrugerCrs(120.3), 'EPSG:4549');
  assert.equal(gp.gaussKrugerCrs(120.3, { zonePrefix: true }), 'EPSG:4528');
  assert.equal(gp.utmCrs(116, 40), 'EPSG:32650');
  assert.equal(gp.normalizeCrs('gcj-02'), 'GCJ02');
  assert.throws(() => gp.normalizeCrs('EPSG:1234'), /unsupported CRS/);

  const arr = new Float64Array([120, 30, 5, 121, 31, 6]);
  const same = gp.transformCoords(arr, 'WGS84', 'EPSG:4549', 3);
  assert.equal(same, arr);
  assert.equal(arr[2], 5);
  assert.ok(arr[0] > 400000 && arr[0] < 600000);
  gp.transformCoords(arr, 'EPSG:4549', 'WGS84', 3);
  assert.ok(Math.abs(arr[3] - 121) < 1e-9);

  const custom = { proj: 'tmerc', lon0: 120, x0: 500000, ellps: 'CGCS2000' };
  const p1 = gp.convert([120.5, 30.5], 'WGS84', custom);
  const p2 = gp.convert([120.5, 30.5], 'WGS84', 'EPSG:4549');
  assert.ok(Math.abs(p1[0] - p2[0]) < 1e-9 && Math.abs(p1[1] - p2[1]) < 1e-9);

  const f = { type: 'Feature', id: 'x', properties: { a: 1 }, geometry: { type: 'Point', coordinates: [120, 30, 9] } };
  const t = gp.transform(f, 'WGS84', 'BD09');
  assert.equal(t.id, 'x');
  assert.equal(t.geometry.coordinates[2], 9);
});

test('point in polygon', () => {
  const poly = { type: 'Polygon', coordinates: [[[0, 0], [2, 0], [2, 2], [0, 2], [0, 0]]] };
  assert.equal(gp.booleanPointInPolygon([1, 1], poly), true);
  assert.equal(gp.booleanPointInPolygon([2, 1], poly), true);
  assert.equal(gp.booleanPointInPolygon([2, 1], poly, { ignoreBoundary: true }), false);
  assert.equal(gp.booleanPointInPolygon([3, 1], poly), false);
});

test('errors surface as exceptions', () => {
  assert.throws(() => gp.distance(BJ, SH, { units: 'radians' }), /unsupported unit/);
  assert.throws(() => gp.intersect({ type: 'LineString', coordinates: [[0, 0], [1, 1]] }, { type: 'Point', coordinates: [0, 0] }), /overlay expects/);
});
