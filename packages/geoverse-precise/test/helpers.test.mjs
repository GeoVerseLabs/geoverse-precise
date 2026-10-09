import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as gp from '../dist/index.js';

// The helper layer is pure GeoJSON bookkeeping: no init() needed.

test('constructors validate and close rings', () => {
  const p = gp.point([1, 2], { a: 1 }, { id: 'x' });
  assert.deepEqual(p, { type: 'Feature', properties: { a: 1 }, geometry: { type: 'Point', coordinates: [1, 2] }, id: 'x' });
  assert.throws(() => gp.point([1]), /at least 2/);
  assert.throws(() => gp.point(['a', 'b']), /must contain numbers/);
  assert.throws(() => gp.lineString([[0, 0]]), /two or more/);
  assert.throws(() => gp.polygon([[[0, 0], [1, 0], [0, 0]]]), /four or more/);

  // an unclosed ring is closed for you
  const poly = gp.polygon([[[0, 0], [1, 0], [1, 1], [0, 0]]]);
  const ring = poly.geometry.coordinates[0];
  assert.deepEqual(ring[0], ring[ring.length - 1]);

  assert.equal(gp.points([[0, 0], [1, 1]]).features.length, 2);
  assert.equal(gp.lineStrings([[[0, 0], [1, 1]]]).features.length, 1);
  assert.equal(gp.polygons([[[[0, 0], [1, 0], [1, 1], [0, 0]]]]).features.length, 1);
  assert.equal(gp.multiPoint([[0, 0], [1, 1]]).geometry.type, 'MultiPoint');
  assert.equal(gp.multiLineString([[[0, 0], [1, 1]]]).geometry.type, 'MultiLineString');
  assert.equal(gp.multiPolygon([[[[0, 0], [1, 0], [1, 1], [0, 0]]]]).geometry.type, 'MultiPolygon');
  assert.equal(gp.geometryCollection([{ type: 'Point', coordinates: [0, 0] }]).geometry.geometries.length, 1);
  assert.equal(gp.geometry('Polygon', [[[0, 0], [1, 0], [1, 1], [0, 0]]]).type, 'Polygon');
  assert.equal(gp.feature(null).geometry, null);
});

test('unit conversion round trips and matches known values', () => {
  assert.ok(Math.abs(gp.convertLength(1, 'kilometers', 'meters') - 1000) < 1e-9);
  assert.ok(Math.abs(gp.convertLength(1, 'miles', 'kilometers') - 1.609344) < 1e-9);
  assert.ok(Math.abs(gp.convertLength(1, 'nauticalmiles', 'meters') - 1852) < 1e-6);
  assert.ok(Math.abs(gp.convertLength(1, 'yards', 'meters') - 0.9144) < 1e-9);
  assert.ok(Math.abs(gp.convertLength(1, 'feet', 'meters') - 0.3048) < 1e-4);

  assert.ok(Math.abs(gp.convertArea(1, 'kilometers', 'meters') - 1e6) < 1e-3);
  assert.ok(Math.abs(gp.convertArea(10000, 'meters', 'hectares') - 1) < 1e-9);
  assert.throws(() => gp.convertLength(-1), /positive/);
  assert.throws(() => gp.convertArea(1, 'parsecs'), /invalid original units/);

  assert.ok(Math.abs(gp.degreesToRadians(180) - Math.PI) < 1e-12);
  assert.ok(Math.abs(gp.radiansToDegrees(Math.PI) - 180) < 1e-12);
  assert.ok(Math.abs(gp.lengthToRadians(gp.earthRadius / 1000) - 1) < 1e-12);
  assert.ok(Math.abs(gp.radiansToLength(1, 'meters') - gp.earthRadius) < 1e-9);
  assert.ok(Math.abs(gp.lengthToDegrees(gp.earthRadius / 1000) - 180 / Math.PI) < 1e-9);

  assert.equal(gp.bearingToAzimuth(-45), 315);
  assert.equal(gp.bearingToAzimuth(45), 45);
  assert.equal(gp.azimuthToBearing(315), -45);
  assert.equal(gp.azimuthToBearing(45), 45);

  assert.equal(gp.round(1.2345, 2), 1.23);
  assert.equal(gp.round(1.5), 2);
  assert.throws(() => gp.round(1, -1), /positive integer/);
  assert.equal(gp.isNumber(3), true);
  assert.equal(gp.isNumber('3'), true);
  assert.equal(gp.isNumber([1]), false);
  assert.equal(gp.isNumber(true), false);
  assert.equal(gp.isObject({}), true);
  assert.equal(gp.isObject([]), false);
});

test('invariant accessors', () => {
  assert.deepEqual(gp.getCoord([1, 2]), [1, 2]);
  assert.deepEqual(gp.getCoord({ type: 'Point', coordinates: [1, 2] }), [1, 2]);
  assert.deepEqual(gp.getCoord(gp.point([1, 2])), [1, 2]);
  assert.throws(() => gp.getCoord({ type: 'LineString', coordinates: [[0, 0], [1, 1]] }), /Point/);

  assert.deepEqual(gp.getCoords(gp.lineString([[0, 0], [1, 1]])), [[0, 0], [1, 1]]);
  assert.equal(gp.getGeom(gp.point([0, 0])).type, 'Point');
  assert.equal(gp.getType(gp.point([0, 0])), 'Point');
  assert.equal(gp.getType(gp.featureCollection([])), 'FeatureCollection');

  assert.doesNotThrow(() => gp.geojsonType({ type: 'Point', coordinates: [0, 0] }, 'Point', 'fn'));
  assert.throws(() => gp.geojsonType({ type: 'Point', coordinates: [0, 0] }, 'LineString', 'fn'), /must be a LineString/);
  assert.doesNotThrow(() => gp.featureOf(gp.point([0, 0]), 'Point', 'fn'));
  assert.throws(() => gp.featureOf(gp.point([0, 0]), 'Polygon', 'fn'), /must be a Polygon/);
  assert.doesNotThrow(() => gp.collectionOf(gp.points([[0, 0]]), 'Point', 'fn'));
  assert.throws(() => gp.collectionOf(gp.points([[0, 0]]), 'Polygon', 'fn'), /must be a Polygon/);
  assert.equal(gp.containsNumber([1, 2]), true);
  assert.equal(gp.containsNumber([[1, 2]]), true);
});

const poly = gp.polygon([
  [[0, 0], [2, 0], [2, 2], [0, 2], [0, 0]],
  [[0.5, 0.5], [1.5, 0.5], [1.5, 1.5], [0.5, 1.5], [0.5, 0.5]],
]);
const multi = gp.multiPolygon([
  [[[0, 0], [1, 0], [1, 1], [0, 0]]],
  [[[3, 3], [4, 3], [4, 4], [3, 3]]],
]);

test('coordEach counts and excludes wrap coordinates', () => {
  assert.equal(gp.coordAll(poly).length, 10);
  let n = 0;
  gp.coordEach(poly, () => {
    n++;
  }, true);
  assert.equal(n, 8, 'two rings, closing position skipped');

  // geometryIndex tracks the ring, multiFeatureIndex the part
  const rings = new Set();
  gp.coordEach(poly, (_c, _ci, _fi, _mi, gi) => {
    rings.add(gi);
  });
  assert.deepEqual([...rings].sort(), [0, 1]);

  const parts = new Set();
  gp.coordEach(multi, (_c, _ci, _fi, mi) => {
    parts.add(mi);
  });
  assert.deepEqual([...parts].sort(), [0, 1]);

  // an early false stops the walk
  let seen = 0;
  gp.coordEach(poly, () => {
    seen++;
    return seen < 3 ? undefined : false;
  });
  assert.equal(seen, 3);

  assert.equal(gp.coordReduce(poly, (sum, c) => sum + c[0], 0), gp.coordAll(poly).reduce((s, c) => s + c[0], 0));
});

test('feature, geom, flatten, line and segment walks', () => {
  const fc = gp.featureCollection([gp.point([0, 0], { a: 1 }), gp.point([1, 1], { a: 2 })]);
  const ids = [];
  gp.featureEach(fc, (f, i) => {
    ids.push(i);
  });
  assert.deepEqual(ids, [0, 1]);
  assert.equal(gp.featureReduce(fc, (s, f) => s + f.properties.a, 0), 3);

  const props = [];
  gp.propEach(fc, (p) => props.push(p.a));
  assert.deepEqual(props, [1, 2]);
  assert.equal(gp.propReduce(fc, (s, p) => s + p.a, 0), 3);

  const types = [];
  gp.geomEach(multi, (g) => types.push(g.type));
  assert.deepEqual(types, ['MultiPolygon']);
  assert.equal(gp.geomReduce(multi, (s) => s + 1, 0), 1);

  // flatten splits the multi into its parts
  const flat = [];
  gp.flattenEach(multi, (f, fi, mi) => flat.push([f.geometry.type, mi]));
  assert.deepEqual(flat, [['Polygon', 0], ['Polygon', 1]]);
  assert.equal(gp.flattenReduce(multi, (s) => s + 1, 0), 2);

  // a polygon with a hole is two lines and eight segments
  const lines = [];
  gp.lineEach(poly, (l, _fi, _mi, gi) => lines.push([l.geometry.coordinates.length, gi]));
  assert.deepEqual(lines, [[5, 0], [5, 1]]);
  assert.equal(gp.lineReduce(poly, (s) => s + 1, 0), 2);

  let segs = 0;
  gp.segmentEach(poly, () => {
    segs++;
  });
  assert.equal(segs, 8);
  assert.equal(gp.segmentReduce(poly, (s) => s + 1, 0), 8);

  const seg = gp.findSegment(poly, { segmentIndex: 0 });
  assert.deepEqual(seg.geometry.coordinates, [[0, 0], [2, 0]]);
  const last = gp.findSegment(poly, { geometryIndex: 1, segmentIndex: -1 });
  assert.deepEqual(last.geometry.coordinates[1], [0.5, 0.5]);
  const first = gp.findPoint(poly, { coordIndex: 0 });
  assert.deepEqual(first.geometry.coordinates, [0, 0]);
});

test('clusters, clone, combine and winding', () => {
  const fc = gp.featureCollection([
    gp.point([0, 0], { cluster: 0 }),
    gp.point([1, 1], { cluster: 1 }),
    gp.point([2, 2], { cluster: 0 }),
  ]);
  assert.equal(gp.getCluster(fc, { cluster: 0 }).features.length, 2);
  assert.equal(gp.getCluster(fc, 'cluster').features.length, 3);

  const sizes = [];
  gp.clusterEach(fc, 'cluster', (c, value) => sizes.push([value, c.features.length]));
  assert.deepEqual(sizes, [[0, 2], [1, 1]]);
  assert.equal(gp.clusterReduce(fc, 'cluster', (s, c) => s + c.features.length, 0), 3);

  const copy = gp.clone(fc);
  copy.features[0].geometry.coordinates[0] = 99;
  assert.equal(fc.features[0].geometry.coordinates[0], 0, 'clone must be deep');

  const combined = gp.combine(
    gp.featureCollection([gp.point([0, 0]), gp.point([1, 1]), gp.lineString([[0, 0], [1, 1]])]),
  );
  const kinds = combined.features.map((f) => f.geometry.type).sort();
  assert.deepEqual(kinds, ['MultiLineString', 'MultiPoint']);
  assert.equal(combined.features.find((f) => f.geometry.type === 'MultiPoint').geometry.coordinates.length, 2);

  // clockwise in lon/lat, the convention turf uses
  const cw = [[0, 0], [0, 1], [1, 1], [1, 0], [0, 0]];
  const ccw = [...cw].reverse();
  assert.equal(gp.booleanClockwise(cw), true);
  assert.equal(gp.booleanClockwise(ccw), false);
  assert.equal(gp.booleanCounterClockwise(ccw), true);
  assert.equal(gp.booleanClockwise(gp.lineString(cw)), true);

  const s = gp.sample(fc, 2);
  assert.equal(s.features.length, 2);
  assert.notEqual(s.features[0], s.features[1]);
  assert.equal(gp.sample(fc, 99).features.length, 3, 'never more than there are');
});

test('random generators stay inside their bbox', () => {
  const b = [10, 20, 11, 21];
  const rp = gp.randomPoint(50, { bbox: b });
  assert.equal(rp.features.length, 50);
  for (const f of rp.features) {
    const [x, y] = f.geometry.coordinates;
    assert.ok(x >= 10 && x <= 11 && y >= 20 && y <= 21, `${x},${y}`);
  }
  const rl = gp.randomLineString(3, { bbox: b, num_vertices: 5 });
  assert.equal(rl.features.length, 3);
  assert.equal(rl.features[0].geometry.coordinates.length, 5);
  const rg = gp.randomPolygon(2, { bbox: b, num_vertices: 6 });
  assert.equal(rg.features.length, 2);
  const ring = rg.features[0].geometry.coordinates[0];
  assert.equal(ring.length, 7);
  assert.deepEqual(ring[0], ring[6]);
  const pos = gp.randomPosition(b);
  assert.ok(pos[0] >= 10 && pos[0] <= 11);
});

test('property filters and small utilities', () => {
  const props = { a: 1, b: 'x', c: true };
  assert.equal(gp.applyFilter(props, 'a'), true);
  assert.equal(gp.applyFilter(props, 'zz'), false);
  assert.equal(gp.applyFilter(props, { a: 1, b: 'x' }), true);
  assert.equal(gp.applyFilter(props, { a: 2 }), false);
  assert.equal(gp.applyFilter(props, ['a', 'b']), true);
  assert.equal(gp.applyFilter(props, ['a', 'zz']), false);
  assert.equal(gp.applyFilter(undefined, 'a'), false);
  assert.equal(gp.propertiesContainsFilter(props, { c: true }), true);

  assert.deepEqual(gp.filterProperties(props, ['a', 'c']), { a: 1, c: true });
  assert.deepEqual(gp.filterProperties(props, []), {});
  assert.deepEqual(gp.filterProperties(props), {});

  const fc = gp.featureCollection([
    gp.point([0, 0], { k: 'a' }),
    gp.point([1, 1], { k: 'b' }),
    gp.point([2, 2], { k: 'a' }),
  ]);
  assert.deepEqual(gp.createBins(fc, 'k'), { a: [0, 2], b: [1] });

  const cloned = gp.cloneProperties(props);
  cloned.a = 99;
  assert.equal(props.a, 1);

  const boxed = { ...gp.featureCollection([gp.point([0, 0], {}, { bbox: [0, 0, 0, 0] })]), bbox: [0, 0, 0, 0] };
  gp.removeBbox(boxed);
  assert.equal(boxed.bbox, undefined);
  assert.equal(boxed.features[0].bbox, undefined);

  assert.doesNotThrow(() => gp.validateBBox([0, 0, 1, 1]));
  assert.throws(() => gp.validateBBox([0, 0, 1]), /4 or 6/);
  assert.throws(() => gp.validateBBox('nope'), /must be an Array/);
  assert.doesNotThrow(() => gp.validateId('a'));
  assert.throws(() => gp.validateId({}), /number or a string/);
});

test('geojsonRbush finds the overlaps and nothing else', () => {
  const tree = gp.geojsonRbush();
  const boxes = [];
  for (let i = 0; i < 40; i++) {
    for (let j = 0; j < 5; j++) {
      const f = gp.polygon([[[i, j], [i + 0.5, j], [i + 0.5, j + 0.5], [i, j + 0.5], [i, j]]], { i, j });
      boxes.push(f);
    }
  }
  tree.load(boxes);
  assert.equal(tree.all().features.length, 200);

  const query = [2, 2, 3, 3];
  const hits = tree.search(query);
  // the grid index must agree with a plain scan
  const overlap = (f) => {
    let [ax0, ay0, ax1, ay1] = [Infinity, Infinity, -Infinity, -Infinity];
    gp.coordEach(f, (c) => {
      ax0 = Math.min(ax0, c[0]);
      ay0 = Math.min(ay0, c[1]);
      ax1 = Math.max(ax1, c[0]);
      ay1 = Math.max(ay1, c[1]);
    });
    return !(query[0] > ax1 || query[2] < ax0 || query[1] > ay1 || query[3] < ay0);
  };
  const scan = boxes.filter(overlap);
  assert.equal(hits.features.length, scan.length, `${hits.features.length} vs ${scan.length}`);
  assert.deepEqual(
    hits.features.map((f) => `${f.properties.i},${f.properties.j}`).sort(),
    scan.map((f) => `${f.properties.i},${f.properties.j}`).sort(),
  );

  assert.equal(tree.collides(query), true);
  assert.equal(tree.collides([100, 100, 101, 101]), false);
  assert.equal(tree.search([100, 100, 101, 101]).features.length, 0);

  // insert, remove, round trip
  const extra = gp.polygon([[[100, 100], [100.5, 100], [100.5, 100.5], [100, 100.5], [100, 100]]], { tag: 'extra' });
  tree.insert(extra);
  assert.equal(tree.collides([100, 100, 101, 101]), true);
  tree.remove(extra);
  assert.equal(tree.collides([100, 100, 101, 101]), false);

  const json = tree.toJSON();
  const rebuilt = gp.geojsonRbush().fromJSON(json);
  assert.equal(rebuilt.search(query).features.length, scan.length);

  // a geometry works as the query too, not just a bbox array
  const asGeom = tree.search({ type: 'Polygon', coordinates: [[[2, 2], [3, 2], [3, 3], [2, 3], [2, 2]]] });
  assert.equal(asGeom.features.length, scan.length);

  tree.clear();
  assert.equal(tree.all().features.length, 0);
  assert.equal(tree.collides(query), false);
});
